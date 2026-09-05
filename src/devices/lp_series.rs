//! Display module for:
//! - LP240
//! - LP360

use super::{checksum, Ctx, DisplayDriver, Mode};
use crate::device::Device;
use crate::error::Result;
use crate::monitor::{Cpu, EnergySample, Gpu};

/// Helper module for the LP Series.
mod dot_matrix {
    pub enum Unit {
        Percent,
        Celsius,
        Fahrenheit,
        Watt,
        Empty,
    }

    impl Unit {
        /// Returns a 5x5 matrix array representing the unit.
        pub const fn get_pattern(self) -> [[bool; 5]; 5] {
            match self {
                Unit::Percent => [
                    [true, true, false, false, true],
                    [true, true, false, true, false],
                    [false, false, true, false, false],
                    [false, true, false, true, true],
                    [true, false, false, true, true],
                ],
                Unit::Celsius => [
                    [true, false, false, false, false],
                    [false, false, true, true, false],
                    [false, true, false, false, false],
                    [false, true, false, false, false],
                    [false, false, true, true, false],
                ],
                Unit::Fahrenheit => [
                    [true, false, true, true, false],
                    [false, false, true, false, false],
                    [false, false, true, true, false],
                    [false, false, true, false, false],
                    [false, false, true, false, false],
                ],
                Unit::Watt => [
                    [false, false, false, false, false],
                    [true, false, true, false, true],
                    [true, false, true, false, true],
                    [true, false, true, false, true],
                    [false, true, false, true, false],
                ],
                Unit::Empty => [[false; 5]; 5],
            }
        }
    }

    /// Returns a 3x5 matrix array representing the number.
    pub const fn get_number_pattern(num: u8) -> [[bool; 3]; 5] {
        match num {
            0 => [
                [true, true, true],
                [true, false, true],
                [true, false, true],
                [true, false, true],
                [true, true, true],
            ],
            1 => [
                [false, true, false],
                [true, true, false],
                [false, true, false],
                [false, true, false],
                [true, true, true],
            ],
            2 => [
                [true, true, true],
                [false, false, true],
                [false, true, false],
                [true, false, false],
                [true, true, true],
            ],
            3 => [
                [true, true, true],
                [false, false, true],
                [true, true, true],
                [false, false, true],
                [true, true, true],
            ],
            4 => [
                [true, false, true],
                [true, false, true],
                [true, true, true],
                [false, false, true],
                [false, false, true],
            ],
            5 => [
                [true, true, true],
                [true, false, false],
                [true, true, true],
                [false, false, true],
                [true, true, true],
            ],
            6 => [
                [true, true, true],
                [true, false, false],
                [true, true, true],
                [true, false, true],
                [true, true, true],
            ],
            7 => [
                [true, true, true],
                [false, false, true],
                [false, true, false],
                [false, true, false],
                [false, true, false],
            ],
            8 => [
                [true, true, true],
                [true, false, true],
                [true, true, true],
                [true, false, true],
                [true, true, true],
            ],
            9 => [
                [true, true, true],
                [true, false, true],
                [true, true, true],
                [false, false, true],
                [true, true, true],
            ],
            _ => [[false; 3]; 5],
        }
    }

    /// Inserts a pattern into the 14x14 matrix at the defined position.
    pub fn insert_pattern<const M: usize, const N: usize>(
        matrix: &mut [[bool; 14]; 14],
        pattern: [[bool; M]; N],
        row_pos: usize,
        col_pos: usize,
    ) {
        // Calculate the actual dimensions that will fit
        let max_rows = (14 - row_pos).min(N);
        let max_cols = (14 - col_pos).min(M);

        // Insert the pattern
        for i in 0..max_rows {
            for j in 0..max_cols {
                matrix[row_pos + i][col_pos + j] = pattern[i][j];
            }
        }
    }

    /// Rotates the matrix values by a given degree.
    pub fn rotate_matrix(matrix: &mut [[bool; 14]; 14], degrees: u16) {
        let mut rotated = [[false; 14]; 14];
        match degrees {
            90 => {
                for i in 0..14 {
                    for j in 0..14 {
                        rotated[j][13 - i] = matrix[i][j];
                    }
                }
            }
            180 => {
                for i in 0..14 {
                    for j in 0..14 {
                        rotated[13 - i][13 - j] = matrix[i][j];
                    }
                }
            }
            270 => {
                for i in 0..14 {
                    for j in 0..14 {
                        rotated[13 - j][i] = matrix[i][j];
                    }
                }
            }
            _ => {
                return;
            }
        }
        *matrix = rotated;
    }

    /// Converts the 14x14 matrix to be data bytes.
    pub fn matrix_to_bytes(matrix: [[bool; 14]; 14]) -> [u8; 28] {
        let mut bytes: [u8; 28] = [0; 28];

        // Values for each row position (HEX: 10, 20, 40, 80, 01, 02, 04)
        const ROW_VALUES: [u8; 7] = [16, 32, 64, 128, 1, 2, 4];

        // First 14 bytes (odd rows)
        for col in 0..14 {
            let mut byte = 0;
            for row_id in 0..7 {
                if matrix[row_id * 2][col] {
                    byte += ROW_VALUES[row_id];
                }
            }
            bytes[col] = byte;
        }

        // Last 14 bytes (even rows - reversed)
        for col in 0..14 {
            let mut byte = 0;
            for row_id in 0..7 {
                if matrix[row_id * 2 + 1][col] {
                    byte += ROW_VALUES[row_id];
                }
            }
            bytes[27 - col] = byte;
        }

        bytes
    }
}

const MODES: &[Mode] = &[
    Mode::CpuUsage,
    Mode::CpuTemperature,
    Mode::CpuPower,
    Mode::GpuUsage,
    Mode::GpuTemperature,
    Mode::GpuPower,
];

pub struct Display {
    cpu: Cpu,
    gpu: Gpu,
    secondary: Option<Mode>,
    fahrenheit: bool,
    rotation: u16,
    last_energy: Option<EnergySample>,
}

pub fn build(ctx: Ctx) -> Result<Box<dyn DisplayDriver>> {
    let secondary = match ctx.secondary {
        Mode::Default => None,
        other => Some(other),
    };
    Ok(Box::new(Display {
        cpu: ctx.cpu,
        gpu: ctx.gpu,
        secondary,
        fahrenheit: ctx.fahrenheit,
        rotation: ctx.rotation,
        last_energy: None,
    }))
}

impl DisplayDriver for Display {
    fn modes(&self) -> &'static [Mode] {
        MODES
    }

    fn secondary_modes(&self) -> &'static [Mode] {
        MODES
    }

    fn warn(&self, mode: Mode) {
        let uses = |m: Mode| mode == m || self.secondary == Some(m);
        if uses(Mode::CpuTemperature) {
            self.cpu.warn_temp();
        }
        if uses(Mode::CpuPower) {
            self.cpu.warn_rapl();
        }
        if uses(Mode::GpuUsage) || uses(Mode::GpuTemperature) || uses(Mode::GpuPower) {
            self.gpu.warn_missing();
        }
    }

    fn init(&mut self, _device: &mut dyn Device) -> Result<()> {
        self.last_energy = self.cpu.read_energy();
        Ok(())
    }

    fn frame(&mut self, device: &mut dyn Device, mode: Mode) -> Result<()> {
        let packet = self.status_message(mode);
        self.last_energy = self.cpu.read_energy();
        device.write_report(&packet)
    }
}

impl Display {
    /// Reads every sensor the panel can show, once.
    ///
    /// Both matrix rows read from this snapshot: calling the getters per row
    /// asked `Cpu::get_usage` twice inside one sampling window, and the second
    /// call correctly refuses to answer — which rendered the second row as 0.
    fn sample(&self) -> crate::monitor::Sample {
        crate::monitor::Sample {
            cpu_usage: self.cpu.get_usage(),
            cpu_temp: self.cpu.get_temp(self.fahrenheit),
            cpu_power: self.cpu.get_power(self.last_energy),
            gpu_usage: self.gpu.get_usage(),
            gpu_temp: self.gpu.get_temp(self.fahrenheit),
            gpu_power: self.gpu.get_power(),
            ..crate::monitor::Sample::default()
        }
    }
}

impl Display {
    fn status_message(&self, mode: Mode) -> [u8; 64] {
        let sample = self.sample();
        let mut data = [0u8; 64];
        data[0] = 16;
        data[1] = 104;
        data[2] = 1;
        data[3] = 5;
        data[4] = 29;
        data[5] = 1;

        let mut matrix = [[false; 14]; 14];
        match self.secondary {
            Some(secondary) => {
                self.insert_data_to_matrix(&mut matrix, 1, self.get_system_info(mode, &sample));
                self.insert_data_to_matrix(&mut matrix, 8, self.get_system_info(secondary, &sample));
            }
            None => {
                self.insert_data_to_matrix(&mut matrix, 5, self.get_system_info(mode, &sample));
            }
        }
        dot_matrix::rotate_matrix(&mut matrix, self.rotation);
        data[6..=33].copy_from_slice(&dot_matrix::matrix_to_bytes(matrix));

        data[34] = checksum(&data, 33);
        data[35] = 22;

        data
    }

    fn get_system_info(&self, mode: Mode, s: &crate::monitor::Sample) -> (u16, dot_matrix::Unit) {
        let unit = if self.fahrenheit {
            dot_matrix::Unit::Fahrenheit
        } else {
            dot_matrix::Unit::Celsius
        };
        match mode {
            Mode::CpuUsage => (s.cpu_usage.unwrap_or(0) as u16, dot_matrix::Unit::Percent),
            Mode::CpuTemperature => (s.cpu_temp.unwrap_or(0) as u16, unit),
            Mode::CpuPower => (s.cpu_power.unwrap_or(0), dot_matrix::Unit::Watt),
            Mode::GpuUsage => (s.gpu_usage.unwrap_or(0) as u16, dot_matrix::Unit::Percent),
            Mode::GpuTemperature => (s.gpu_temp.unwrap_or(0) as u16, unit),
            Mode::GpuPower => (s.gpu_power.unwrap_or(0), dot_matrix::Unit::Watt),
            _ => (0, dot_matrix::Unit::Empty),
        }
    }

    fn insert_data_to_matrix(&self, matrix: &mut [[bool; 14]; 14], row_id: usize, data: (u16, dot_matrix::Unit)) {
        let (value, unit) = data;
        // The panel is three digits wide; clamp rather than rendering a bogus
        // leading digit for anything larger.
        let value = value.min(999);
        if value < 100 {
            // 2-digit number
            dot_matrix::insert_pattern(matrix, dot_matrix::get_number_pattern((value / 10) as u8), row_id, 1);
            dot_matrix::insert_pattern(matrix, dot_matrix::get_number_pattern((value % 10) as u8), row_id, 5);
            dot_matrix::insert_pattern(matrix, unit.get_pattern(), 5, 9);
        } else {
            // 3-digit number
            dot_matrix::insert_pattern(matrix, dot_matrix::get_number_pattern((value / 100) as u8), row_id, 1);
            dot_matrix::insert_pattern(matrix, dot_matrix::get_number_pattern((value % 100 / 10) as u8), row_id, 5);
            dot_matrix::insert_pattern(matrix, dot_matrix::get_number_pattern((value % 10) as u8), row_id, 9);
            dot_matrix::insert_pattern(matrix, unit.get_pattern(), 5, 13);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(secondary: Option<Mode>, rotation: u16) -> Display {
        Display {
            cpu: Cpu::new(),
            gpu: Gpu::absent(),
            secondary,
            fahrenheit: false,
            rotation,
            last_energy: None,
        }
    }

    #[test]
    fn packet_header_and_terminator_match_the_protocol_table() {
        let pkt = display(None, 0).status_message(Mode::CpuUsage);
        assert_eq!([pkt[0], pkt[1], pkt[2], pkt[3], pkt[4], pkt[5]], [16, 104, 1, 5, 29, 1]);
        assert_eq!(pkt[34], checksum(&pkt, 33));
        assert_eq!(pkt[35], 22);
    }

    #[test]
    fn a_four_digit_value_clamps_instead_of_drawing_a_blank_digit() {
        // get_number_pattern returns an empty glyph for anything above 9, so an
        // unclamped 1200 W used to render as a blank followed by "00".
        let d = display(None, 0);
        let mut matrix = [[false; 14]; 14];
        d.insert_data_to_matrix(&mut matrix, 5, (1200, dot_matrix::Unit::Watt));
        let lit = matrix.iter().flatten().filter(|p| **p).count();
        assert!(lit > 0, "clamped value must still draw digits");
    }

    #[test]
    fn rotation_preserves_the_number_of_lit_pixels() {
        let d = display(None, 0);
        let mut base = [[false; 14]; 14];
        d.insert_data_to_matrix(&mut base, 5, (42, dot_matrix::Unit::Percent));
        let expected = base.iter().flatten().filter(|p| **p).count();

        for deg in [90u16, 180, 270] {
            let mut m = base;
            dot_matrix::rotate_matrix(&mut m, deg);
            assert_eq!(
                m.iter().flatten().filter(|p| **p).count(),
                expected,
                "rotation by {deg} lost pixels"
            );
        }
    }

    #[test]
    fn matrix_serialises_to_the_28_bytes_the_packet_reserves() {
        let m = [[true; 14]; 14];
        assert_eq!(dot_matrix::matrix_to_bytes(m).len(), 28);
    }
}
