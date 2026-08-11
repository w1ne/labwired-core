# Arduino Nano 33 BLE Sense

Nano 33 BLE form factor with the original **Sense** environmental suite
(LSM9DS1, HTS221, LPS22HB, APDS9960) on internal Wire1, plus PDM mic on the
chip PDM block.

| | |
|--|--|
| Product id | `arduino-nano-33-ble-sense` |
| MPN | ABX00031 (original Sense) |
| System | `configs/systems/arduino-nano-33-ble-sense.yaml` |
| Chip | `configs/chips/nrf52840.yaml` |
| PIO | `nano33ble` (same Arduino mbed target as base Nano 33 BLE) |
| Sensor bus | Wire1 → **`i2c0`** (TWI0 @ 0x40003000) |
| E2E marker | `LW_SENSE_OK` |

## Overlay composition

```
arduino-nano-33-ble  (carrier LEDs)
  + 5× external_device on i2c0  (Wire1 first-begin instance)
  = arduino-nano-33-ble-sense
```

Arduino mbed assigns TWI instance IDs at `WireN.begin()` when `PinMap_I2C` is
empty: the first bus that begins is instance 0 = TWI0 = chip id **`i2c0`**.
Sense sketches call `Wire1.begin()` only, so onboard sensors must attach to
`i2c0`, **not** `twi1` (TWI1 @ 0x40004000).

## Onboard sensors (Wire1 / `i2c0`)

| Device | type | Address | SimInput |
|--------|------|---------|----------|
| LSM9DS1 AG | `lsm9ds1_ag` | 0x6B | ax/ay/az (g), gx/gy/gz (°/s) |
| LSM9DS1 Mag | `lsm9ds1_m` | 0x1E | mx/my/mz (gauss) |
| HTS221 | `hts221` | 0x5F | humidity (%), temperature (°C) |
| LPS22HB | `lps22hb` | 0x5C | pressure (hPa), temperature (°C) |
| APDS9960 | `apds9960` | 0x39 | proximity, ambient |
| MP34DT05 mic | chip `pdm` | — | PDM peripheral (no external_device) |

Firmware must raise **sensors 3V3** (P0.22) and **I2C pull-ups** (P1.00) before
Wire1 — firmware-owned GPIO, not phantom board_io.

## End-to-end verify

```bash
# Build sketch
cd examples/arduino-nano-33-ble-sense/verify && pio run -e nano33ble

# Sim (from core/, LabWired volume target recommended)
labwired test --script examples/arduino-nano-33-ble-sense/verify/test.yaml \
  --output-dir /tmp/sense-ok --max-steps 3000000
# Expect uart: LW_SENSE_BOOT then LW_SENSE_OK
```

Arduino matrix: board `arduino-nano-33-ble-sense` × sketch `L4_sense_whoami`
(marker `LW_SENSE_OK`).

## LEDs

Same as [Arduino Nano 33 BLE](../arduino-nano-33-ble/README.md).

## Not modelled

- Sense **Rev2** BMI270/BMM150/HS3003 swap — separate future product id.
- Gesture FIFO, HTS221 factory calibration, FS-aware IMU encode.
- PDM mic capture path (chip block present; no E2E sketch yet).
- Silicon capture of this PCB (rides nRF52840 die evidence).

## Tests

```bash
cargo test -p labwired-core --lib nano33ble -- --nocapture
```
