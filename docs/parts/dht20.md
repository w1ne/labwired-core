# DHT20

Aosong DHT20 I²C temperature and humidity module (AHT20 inside).

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `dht20` |
| Bus | I²C |
| Address | 0x38 (fixed) |
| Source | [`configs/devices/dht20.yaml`](../../configs/devices/dht20.yaml) (declarative, no Rust model) |
| Datasheet | Aosong *DHT20 Humidity and Temperature Module* manual V1.0, §6 signal transformation, §7 sensor reading process |
| Tier | smoke-proven (register-level tests: `crates/core/tests/everyday_sensor_set.rs`) |

## Pins / attachment

| Pin / net | Role |
|-----------|------|
| SDA | I²C SDA |
| SCL | I²C SCL |
| VDD/GND | Power (digital twin) |

## Support matrix

| Behavior | Status | Notes |
|----------|--------|-------|
| 0x71 status (0x18 = calibrated, idle) | ✅ | Status is the first byte of the frame |
| 0xAC 0x33 0x00 trigger → 7-byte frame | ✅ | 20-bit RH + 20-bit T + CRC-8 over six bytes, after 80 ms |
| BUSY during measurement | ✅ | Reads 0x98 until 80 ms elapse |
| Calibration-register init (0x1B/0x1C/0x1E) | ❌ | Part always reads calibrated |
| Analog / power | ❌ | Bench |

## Stimulus

Host inputs: `temperature` (°C), `humidity` (%RH).

## How to run

1. Place the part and an MCU in Playground, or add it to a system YAML:
   ```yaml
   external_devices:
     - id: "dht20"
       type: "dht20"
       connection: "i2c0"   # the bus id your chip system uses
   ```
2. Agent: `labwired_list` → `labwired_describe id=dht20` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. Register-level smoke: `cargo test -p labwired-core --test everyday_sensor_set` — 0x71 → 0x18; trigger + 80 ms → status 0x18, RH 0x80000, T 0x60000, CRC valid. The same file's negative control proves the check fails against a part that answers only 0x00 or only 0xFF.

## Limitations

- Bytes after the 0x71 status byte are the live measurement, not the last one.
- A bare read with no command answers 0xFF.

See the YAML header for the full datasheet trace and every named approximation.

## Related

- [Parts index](index.md)
- [Onboard a part](../howto/onboard-part.md)
- [Fidelity](../fidelity.md)
