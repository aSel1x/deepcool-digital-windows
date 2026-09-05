//! CRC-16 helpers for the DeepCool LM protocol, plus a brute-force parameter
//! search used when reverse-engineering a new device.

use crc_any::CRCu16;

/// Результат поиска CRC-16 параметров.
pub struct CrcResult {
    pub poly: u16,
    pub init: u16,
    pub xorout: u16,
    pub refin: bool,
    /// `true` when the device transmits the checksum little-endian.
    pub swapped: bool,
}

/// Byte order a candidate parameter set was matched with.
#[derive(Clone, Copy, PartialEq)]
enum ByteOrder {
    Direct,
    Swapped,
}

/// Запускает brute-force поиск параметров CRC-16.
///
/// Принимает список пар (data, expected_crc) и возвращает параметры, совпавшие
/// для **всех** пар при **одном и том же** порядке байт — раньше порядок
/// выбирался для каждой пары независимо, из-за чего проходили смешанные наборы.
pub fn brute_force_crc(pairs: &[(&str, &str)]) -> Option<CrcResult> {
    let parsed: Vec<(Vec<u8>, u16)> = pairs
        .iter()
        .map(|(data_hex, crc_hex)| {
            let data = hex_bytes(data_hex)?;
            let crc = u16::from_str_radix(crc_hex, 16).ok()?;
            Some((data, crc))
        })
        .collect::<Option<_>>()?;

    if parsed.is_empty() {
        return None;
    }

    let common_polys = [0x1021u16, 0x8005, 0x8408, 0xA001, 0xC002, 0x0589];

    for &poly in &common_polys {
        for &xorout in &[0x0000u16, 0xFFFF] {
            for &ref_both in &[true, false] {
                for &order in &[ByteOrder::Direct, ByteOrder::Swapped] {
                    for init in 0u16..=0xFFFF {
                        let all_match = parsed.iter().all(|(data, expected)| {
                            let mut crc_obj = CRCu16::create_crc(poly, 16, init, xorout, ref_both);
                            crc_obj.digest(data);
                            let computed = crc_obj.get_crc();
                            let computed = match order {
                                ByteOrder::Direct => computed,
                                ByteOrder::Swapped => computed.swap_bytes(),
                            };
                            computed == *expected
                        });

                        if all_match {
                            return Some(CrcResult {
                                poly,
                                init,
                                xorout,
                                refin: ref_both,
                                swapped: order == ByteOrder::Swapped,
                            });
                        }
                    }
                }
            }
        }
    }

    None
}

fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&s[i..i + 2], 16).ok()?);
    }
    Some(bytes)
}

/// Вычисляет CRC-16 для пакета DeepCool LM360.
///
/// Это **CRC-16/XMODEM** (poly = 0x1021, init = 0x0000, xorout = 0, без
/// рефлексии) по телу пакета, начиная с индекса 2 — то есть после заголовка
/// `AA <len>`.
///
/// Раньше здесь стоял «init = 0xAA04», задокументированный как константа
/// протокола. Это не константа: 0xAA04 — сами байты заголовка config-пакета, и
/// затравка ими эквивалентна пропуску этих двух байт. Для пакетов другой длины
/// (кадровый заголовок `AA 08 …`, init-запрос `AA 01 …`) значение обязано быть
/// другим, поэтому функция считала неверный CRC для всего, кроме одного пакета.
///
/// Возвращает CRC в **little-endian** порядке, как он идёт по проводу.
pub fn compute_crc16(data: &[u8]) -> [u8; 2] {
    let body = if data.len() >= 2 { &data[2..] } else { data };
    let mut crc = CRCu16::create_crc(0x1021, 16, 0x0000, 0x0000, false);
    crc.digest(body);
    crc.get_crc().to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packets and checksums captured from a real LM360, taken from the
    /// protocol constants in `devices/lm_series.rs` and the `--crc` help text.
    /// `data` is the packet body; `crc` is the trailing checksum, big-endian.
    const VECTORS: &[(&str, u16)] = &[
        // Frame header: trailing bytes are `bc 11` on the wire.
        ("aa08000001005802002c01", 0x11BC),
        // Init query: trailing bytes are `29 91` on the wire.
        ("aa010009", 0x9129),
        // Config packets from the --crc example in the help text.
        ("aa040006026400", 0x8E17),
        ("aa040006046400", 0x3CB7),
    ];

    fn crc_of(hex: &str) -> u16 {
        let bytes = hex_bytes(hex).unwrap();
        u16::from_le_bytes(compute_crc16(&bytes))
    }

    #[test]
    fn matches_every_captured_packet() {
        for (hex, expected) in VECTORS {
            assert_eq!(crc_of(hex), *expected, "checksum mismatch for {hex} (expected {expected:#06x})");
        }
    }

    #[test]
    fn is_not_length_specific() {
        // The old implementation hard-coded the seed 0xAA04, which only ever
        // matched the length-4 config packet. Guard against that regression by
        // asserting the two other lengths explicitly.
        assert_eq!(crc_of("aa08000001005802002c01"), 0x11BC, "frame header (len 0x08)");
        assert_eq!(crc_of("aa010009"), 0x9129, "init query (len 0x01)");
    }

    #[test]
    fn returns_little_endian_bytes() {
        // 0x8E17 goes out as 17 8e.
        assert_eq!(compute_crc16(&hex_bytes("aa040006026400").unwrap()), [0x17, 0x8E]);
    }

    #[test]
    fn short_input_does_not_panic() {
        for hex in ["", "aa", "aa04"] {
            let _ = compute_crc16(&hex_bytes(hex).unwrap());
        }
    }

    #[test]
    fn brute_force_recovers_the_lm_parameters() {
        // Both packets have the same length byte, so a fixed seed exists.
        let pairs = [("aa040006026400", "178e"), ("aa040006046400", "b73c")];
        let found = brute_force_crc(&pairs).expect("parameters should be recoverable");
        assert_eq!(found.poly, 0x1021);
        assert_eq!(found.init, 0xAA04);
        assert_eq!(found.xorout, 0x0000);
        assert!(!found.refin);
        // The expected values above are the little-endian on-wire bytes.
        assert!(found.swapped);
    }

    #[test]
    fn brute_force_rejects_inconsistent_pairs() {
        let pairs = [("aa040006026400", "178e"), ("aa040006046400", "dead")];
        assert!(brute_force_crc(&pairs).is_none());
    }

    #[test]
    fn hex_bytes_rejects_odd_length_and_non_hex() {
        assert!(hex_bytes("abc").is_none());
        assert!(hex_bytes("zz").is_none());
        assert_eq!(hex_bytes("aa04"), Some(vec![0xAA, 0x04]));
    }
}
