# SHT40

Sensirion SHT40 I²C temperature and humidity sensor (±0.2 °C / ±1.8 %RH class).

!!! tip "Live catalog"
    Prefer `labwired_describe` with the catalog id over guessing pins or addresses.

## Status at a glance

| Aspect | Status |
|--------|--------|
| Catalog id | `sht40` |
| Bus | I²C |
| Address | 0x44 (SHT40-AD1B); 0x45 / 0x46 for the -BD1B / -CD1B variants |
| Source | [`configs/devices/sht40.yaml`](../../configs/devices/sht40.yaml) (declarative, no Rust model) |
| Datasheet | Sensirion *Datasheet SHT4x* (v6.x): I2C command overview, conversion formulas, system timing table |
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
| 0xFD / 0xF6 / 0xE0 measure (high/med/low precision) | ✅ | T + RH words with CRC-8; ready after the datasheet MAX 8.3 / 4.5 / 1.6 ms, 0xFF before |
| 0x89 serial number | ✅ | Constant serial, CRC-framed |
| 0x94 soft reset | ⚠️ | Accepted; no state to clear |
| Heater commands (0x39/0x32/0x2F/0x24/0x1E/0x15) | ⚠️ | Take the 1.1 s / 0.11 s heater time, then measure; no heating |
| Repeatability noise | ❌ | All precisions answer the exact stimulus |
| Analog / power | ❌ | Bench |

## Stimulus

Host inputs: `temperature` (°C), `humidity` (%RH).

## How to run

1. Place the part and an MCU in Playground, or add it to a system YAML:
   ```yaml
   external_devices:
     - id: "sht40"
       type: "sht40"
       connection: "i2c0"   # the bus id your chip system uses
   ```
2. Agent: `labwired_list` → `labwired_describe id=sht40` → wire → `labwired_validate` → `labwired_run` / `labwired_verify`.
3. Register-level smoke: `cargo test -p labwired-core --test everyday_sensor_set` — 0xFD → T word 0x6666 at 25 °C, RH word 0x72B0 at 50 %RH, both CRCs valid; serial 0x0F4E 0x2A71. The same file's negative control proves the check fails against a part that answers only 0x00 or only 0xFF.

## Limitations

- Single-byte opcodes, unlike the SHT3x — see [`sht30.yaml`](../../configs/devices/sht30.yaml) for that family.
- Heater effect and repeatability noise not modelled.

See the YAML header for the full datasheet trace and every named approximation.

## Related

- [Parts index](index.md)
- [Onboard a part](../howto/onboard-part.md)
- [Fidelity](../fidelity.md)
