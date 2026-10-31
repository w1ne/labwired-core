# SGP40

Sensirion SGP40 MOx VOC sensor (raw SRAW_VOC for the host-side VOC Index).

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `sgp40` |
| Bus | I²C |
| Address | 0x59 (fixed) |
| Source | [`configs/devices/sgp40.yaml`](../../configs/devices/sgp40.yaml) (declarative, no Rust model) |
| Datasheet | Sensirion *Datasheet SGP40* (v1.x), I2C command overview |
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
| 0x260F measure_raw_signal | ✅ | 2 compensation words in, SRAW_VOC + CRC out after 30 ms |
| 0x280E self-test | ✅ | 0xD400 (all passed) after 320 ms |
| 0x3615 heater off / 0x3682 serial | ⚠️ | Accepted / constant serial |
| MOx physics, RH/T compensation | ❌ | Compensation words are accepted and discarded |
| Analog / power | ❌ | Bench |

## Stimulus

Host inputs: `voc_sraw` (ticks).

## How to run

1. Place the part and an MCU in Playground, or add it to a system YAML:
   ```yaml
   external_devices:
     - id: "sgp40"
       type: "sgp40"
       connection: "i2c0"   # the bus id your chip system uses
   ```
2. Agent: `labwired_list` → `labwired_describe id=sgp40` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. Register-level smoke: `cargo test -p labwired-core --test everyday_sensor_set` — 0x260F → 28000 ticks with valid CRC; 0x280E → 0xD400. The same file's negative control proves the check fails against a part that answers only 0x00 or only 0xFF.

## Limitations

- The VOC Index is computed by Sensirion's Gas Index Algorithm on the host, as on real hardware.

See the YAML header for the full datasheet trace and every named approximation.

## Related

- [Parts index](index.md)
- [Onboard a part](../howto/onboard-part.md)
- [Fidelity](../fidelity.md)
