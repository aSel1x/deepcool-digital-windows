# DeepCool Digital

Cross-platform CLI for DeepCool devices with digital displays (CPU coolers, AIO liquid coolers, PC cases).

Reads CPU/GPU/RAM metrics and renders them to the device display in real time.

**Windows**: [releases](https://github.com/aSel1x/deepcool-digital-windows/releases)
**Linux**: See [Nortank12/deepcool-digital-linux](https://github.com/Nortank12/deepcool-digital-linux)

## Features

- **LCD support** (LM-series) — SVG templates, PNG/JPG/BMP stills and GIF animations rendered to an RGB565 framebuffer
- **Segment display support** — AK, AG, LS, LD, LQ, LP, CH, CH510
- **Cross-platform** — Windows (WinUSB + LHM) and Linux (hidapi + sysfs + NVML)
- **Configurable** — brightness, rotation, mode, temperature unit via `config.toml`
- **CRC reverse-engineering tool** — `--crc` flag for discovering protocol parameters of new devices

## Quick Start

### Windows

1. Download the latest [release](https://github.com/aSel1x/deepcool-digital-windows/releases)
2. Connect your DeepCool device via USB
3. Run `deepcool-digital.exe` **as Administrator**

> **Note**: No drivers needed. Windows auto-installs HID (standard USB) or WinUSB (JUSB2.0 header).

### Linux

```bash
chmod +x deepcool-digital
sudo ./deepcool-digital
```

## Configuration

The config is searched for in this order, first match wins:

1. `--config <path>`
2. `$DEEPCOOL_CONFIG`
3. `./config.toml`
4. `$XDG_CONFIG_HOME/deepcool-digital/config.toml` (Linux) / `%APPDATA%\deepcool-digital\config.toml` (Windows)
5. `/etc/deepcool-digital/config.toml` (Linux)
6. next to the executable

A missing config is fine; a malformed one, or one with an unknown key, is an error. Relative paths resolve against the directory holding the config. Start from [`configs/example.toml`](configs/example.toml) and see [configs/README.md](configs/README.md) for the full schema.

```toml
update_interval_ms = 1000
debug = false

[lcd]
brightness = 100              # 0-100
rotation = 0                  # 0, 90, 180, 270
source = "svg-templates/lm360-dashboard.svg"

[segment]
mode = "auto"                 # auto, cpu_temp, cpu_usage, ...
secondary = "gpu_temp"        # Dual-display devices (CH, LP)
fahrenheit = false
alarm = false

# Windows only
[lhm]
url = "http://127.0.0.1:8085/data.json"
```

Settings your device cannot honour are warned about and ignored rather than silently applied.

### Requirements

- **Windows — all metrics**: [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) with "Run Remote Web Server" enabled. CPU sensors come from it too, not only GPU.
- **Linux NVIDIA**: NVML (included with NVIDIA drivers)
- **Linux AMD/Intel**: sysfs + hwmon (kernel modules: `amdgpu`, `coretemp`, `k10temp`, `zenpower`)
- **Linux CPU power**: RAPL, which is root-only on most kernels since CVE-2020-8694

## Supported Devices

### CPU Air Coolers

| Model | Status |
|-------|--------|
| AK400/AK500/AK500S/AK620 DIGITAL | ✅ |
| AK400/AK500/AK620 DIGITAL PRO | ✅ |
| AK620 G2 / AK700 DIGITAL NYX | ✅ |
| AG400/AG620 DIGITAL | ✅ |
| ASSASSIN IV VC VISION | ✅ |

### CPU Liquid Coolers

| Model | Status |
|-------|--------|
| LD240 / LD360 | ✅ |
| LQ240 / LQ360 | ✅ |
| LS520 / LS720 SE DIGITAL | ✅ |
| LP240 / LP360 | ✔️ Partial |

### Cases

| Model | Status |
|-------|--------|
| CH360 / CH560 / CH510 DIGITAL | ✅ |
| CH170 / CH270 / CH690 DIGITAL | ✔️ Partial |
| MORPHEUS | ✅ |

**✅ Full** &nbsp; **✔️ Partial** &nbsp; **❓ Not added**

> If your device is not listed, run the program and check if it detects it.
> Report results via GitHub issues.

## Command line

```
--config <path>            Use this config file instead of searching
--list                     List attached DeepCool devices and exit
--crc <d1> <c1> <d2> <c2>  CRC-16 brute-force (two packet/CRC pairs)
--send <hex>               Send a raw hex packet and read the response
--help, -h                 Show help
--version, -V              Show version
```

## CRC Reverse Engineering

For adding support to new devices, use the built-in CRC brute-force tool:

```bash
deepcool-digital --crc <data1> <crc1> <data2> <crc2>
```

Example:
```bash
deepcool-digital --crc aa040006026400 178e aa040006046400 b73c
```

This finds CRC-16 parameters (poly, init, reflection) that match both packets.

## Building from Source

### Windows

```powershell
git clone https://github.com/aSel1x/deepcool-digital-windows
cd deepcool-digital-windows
cargo build --release
```

Output: `target\release\deepcool-digital.exe`

### Linux

```bash
sudo apt install build-essential pkg-config libudev-dev
cargo build --release
```

Output: `target/release/deepcool-digital`

## More Information

- [Device List & USB Mapping](device-list/README.md)
- [Configuration Guide](configs/README.md)

## Development

```bash
cargo test                                        # unit + integration tests
cargo clippy --all-targets -- -D warnings         # what CI enforces
cargo test --release                              # the CRC brute-force test is slow in debug
```

Tests run without any hardware: the protocol packet builders, the CRC, the
sensor parsers and the shipped SVG templates are all pure logic, and
`tests/fixtures/lhm-sample.json` is an anonymised LibreHardwareMonitor capture.

## License

GNU GPL v3.0 — see [LICENSE](LICENSE)

Based on [Nortank12/deepcool-digital-linux](https://github.com/Nortank12/deepcool-digital-linux).

### Third-party assets

`configs/assets/` bundles the **Saira** and **Saira Condensed** typefaces by
Omnibus-Type, used under the SIL Open Font License 1.1 — see
[configs/assets/OFL.txt](configs/assets/OFL.txt).
