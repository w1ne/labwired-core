# PMSA003I

Plantower PMSA003I particulate-matter sensor, I²C variant (PM1.0 / PM2.5 / PM10).

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `pmsa003i` |
| Bus | I²C |
| Address | 0x12 (fixed) |
| Source | [`configs/devices/pmsa003i.yaml`](../../configs/devices/pmsa003i.yaml) (declarative, no Rust model) |
| Datasheet | Plantower *PMSA003I series data manual*, I²C register table (0x00..0x1F) |
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
| 32-byte frame (0x42 0x4D, length 28, 13 data words, check code) | ✅ | Bare reads and pointer-0x00 reads both return whole frames (pointer wraps 0x1F → 0x00) |
| Check code | ✅ | Computed byte sum over what the twin sends |
| CF=1 vs atmospheric words | ⚠️ | Report the same values |
| Fan, 1 s cadence, warm-up, SET/RESET pins | ❌ |  |
| Analog / power | ❌ | Bench |

## Stimulus

Host inputs: `pm1_0`, `pm2_5`, `pm10` (µg/m³); `n0_3` … `n10` (particles / 0.1 L).

## How to run

1. Place the part and an MCU in Playground, or add it to a system YAML:
   ```yaml
   external_devices:
     - id: "pmsa003i"
       type: "pmsa003i"
       connection: "i2c0"   # the bus id your chip system uses
   ```
2. Agent: `labwired_list` → `labwired_describe id=pmsa003i` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. Register-level smoke: `cargo test -p labwired-core --test everyday_sensor_set` — Bare 32-byte read: 0x42 0x4D, length 28, PM2.5 = 8, >0.3 µm = 900, check code = byte sum. The same file's negative control proves the check fails against a part that answers only 0x00 or only 0xFF.

## Limitations

- Reserved word reads 0x0000.

See the YAML header for the full datasheet trace and every named approximation.

## Related

- [Parts index](index.md)
- [Onboard a part](../howto/onboard-part.md)
- [Fidelity](../fidelity.md)
