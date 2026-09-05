# Configuration Guide

## Where the config is read from

The first existing file wins:

1. `--config <path>`
2. `$DEEPCOOL_CONFIG`
3. `./config.toml` (working directory)
4. `$XDG_CONFIG_HOME/deepcool-digital/config.toml` (Linux) or `%APPDATA%\deepcool-digital\config.toml` (Windows)
5. `/etc/deepcool-digital/config.toml` (Linux)
6. `config.toml` next to the executable

A missing config is fine — defaults are used, and the program prints which file it loaded. A **malformed** config, or one containing an unknown key, is a hard error: silently ignoring a typo'd setting is worse than stopping.

**Relative paths** (`lcd.source`, `lcd.font_dir`) resolve against the directory holding the config file, then against the executable's directory.

## Structure

```toml
update_interval_ms = 1000   # Frame update interval, 100-2000 ms (clamped)
debug = false               # Enable debug logging

[device]                    # Only needed with several DeepCool devices attached
product_id = 38
vendor_id = 13875
gpu = "nvidia:0"            # Linux: PCI address (0000:01:00.0) or vendor:index

[lcd]                       # LCD devices (LM-series)
brightness = 100            # 0-100 (default: 100)
rotation = 0                # 0, 90, 180, 270 (default: 0)
source = "svg-templates/lm360-dashboard.svg"
fit = "stretch"             # stretch | contain | cover — raster sources only
font_dir = "assets"         # Extra .ttf/.otf faces (default: configs/assets)

[segment]                   # Segment devices (AK, AG, LS, LD, LQ, LP, CH, CH510)
mode = "auto"               # Display mode (see below)
secondary = "gpu_temp"      # Secondary display (CH, LP only)
fahrenheit = false          # Temperature unit
alarm = false               # CPU overheat alarm
rotation = 0                # 0, 90, 180, 270 — LP-series dot matrix only

[lhm]                       # Windows only — LibreHardwareMonitor
url = "http://127.0.0.1:8085/data.json"
timeout_ms = 2000           # Request timeout (default: 2000)
auth_bearer = "..."         # Bearer token (alternative to Basic auth)
auth_basic_user = "user"
auth_basic_pass = "pass"
```

Settings a device cannot honour are **warned about and ignored**, not silently applied. For example the AG-series has no Fahrenheit unit code in its protocol, so `fahrenheit = true` produces a warning and Celsius.

## Segment Modes

Run the program to see the exact list your device supports — it is printed at startup and comes from the driver itself.

| Value | Description | Devices |
|-------|-------------|---------|
| `auto` | Cycle the device's modes every 5 s (or the single mode it has) | All |
| `cpu_temp` | CPU temperature | AK, AG, CH, LS, LP |
| `cpu_usage` | CPU usage % | AK, AG, CH, LP |
| `cpu_power` | CPU power (W) | LS, LP |
| `cpu_freq` | CPU frequency (MHz) | CH gen 2 |
| `gpu_temp` | GPU temperature | CH (secondary), LP |
| `gpu_usage` | GPU usage % | CH (secondary), LP |
| `gpu_power` | GPU power (W) | LP |
| `cpu` | All CPU metrics | CH510 |
| `gpu` | All GPU metrics | CH510, CH gen 2 |

`auto` is valid on every device: with more than one displayable mode it cycles them every 5 s, with exactly one it simply is that mode. Any other unsupported mode is rejected at startup with the list the device does support, and `secondary` is validated the same way.

## LCD Variables for SVG Templates

Every field below is always present, so a template never breaks on a missing sensor. **Undefined variables are an error**, not an empty string — a typo fails loudly instead of rendering a blank attribute.

| Placeholder | Type | Description |
|-------------|------|-------------|
| `{{ cpu_temp }}` | 0-255 | CPU temperature, in the configured unit |
| `{{ gpu_temp }}` | 0-255 | GPU temperature |
| `{{ liquid_temp }}` | 0-255 | Liquid coolant temperature (Windows only) |
| `{{ cpu_usage }}` | 0-100 | CPU usage % |
| `{{ gpu_usage }}` | 0-100 | GPU usage % |
| `{{ ram_usage }}` | 0-100 | RAM usage % |
| `{{ vram_usage }}` | 0-100 | VRAM usage % |
| `{{ cpu_power }}` | u16 | CPU power, W |
| `{{ gpu_power }}` | u16 | GPU power, W |
| `{{ cpu_freq }}` | u16 | CPU frequency, MHz |
| `{{ gpu_freq }}` | u16 | GPU frequency, MHz |
| `{{ vcore }}` | u16 | CPU voltage in **millivolts** (use `{{ (vcore / 1000) \| round(2) }}`) |
| `{{ cpu_fan_rpm }}` | u16 | CPU fan RPM |
| `{{ liquid_fan_rpm }}` | u16 | Pump RPM |
| `{{ sys_fan_rpm }}` | u16 | First system fan RPM |
| `{{ clock }}` | string | Local time, `HH:MM:SS` |
| `{{ bar_track_width }}` | u32 | Usable bar width in pixels for this panel |

### Telling "absent" apart from "zero"

A sensor that could not be read reports **0** in the numeric field. To distinguish that from a genuine zero, use the parallel `valid` map:

```xml
<text>{% if valid.cpu_temp %}{{ cpu_temp }}{% else %}--{% endif %}</text>
```

`valid` has one boolean per numeric field above. Showing a stale or invented number is the failure mode worth designing against, so prefer `--` over `0`.

### Idioms

```xml
<!-- Zero-padded three-digit value -->
{% if cpu_temp < 10 %}00{% elif cpu_temp < 100 %}0{% endif %}{{ cpu_temp }}

<!-- Progress bar scaled to the panel -->
<rect width="{{ cpu_usage * bar_track_width // 100 }}" height="8" fill="white"/>

<!-- Ten-segment RAM meter -->
{% for i in range(10) %}
  <rect x="{{ 65 + i * 24 }}" y="193" width="22" height="22"
        fill="{% if i < ram_usage / 10 %}white{% else %}none{% endif %}"/>
{% endfor %}
```

## Notes

1. Segment displays do not use SVG templates.
2. LCD rotation is applied by the device's hardware orientation, not in software.
3. If `lcd.source` is missing or invalid, the program reports the reason and exits.
4. `[lhm]` is Windows-only; on Linux the metrics come from sysfs, procfs and NVML.
5. Raster sources (PNG/JPG/BMP/GIF) are scaled once at load time. GIFs honour their own frame delays, clamped to a 100 ms minimum.
