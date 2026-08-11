# Arduino Nano 33 BLE Sense

Original Sense revision (ABX00031): Nano 33 BLE carrier + LSM9DS1 / HTS221 /
LPS22HB / APDS9960 on Wire1 and MP34DT05 on PDM.

## Status

| Aspect | Status |
|--------|--------|
| Product id | `arduino-nano-33-ble-sense` |
| Chip yaml | `configs/chips/nrf52840.yaml` |
| System yaml | `configs/systems/arduino-nano-33-ble-sense.yaml` |
| Example | `examples/arduino-nano-33-ble-sense/` |
| PlatformIO | `nano33ble` / Arduino |
| Tier | sim-validated (no dedicated PCB silicon capture) |
| Sensor bus | Wire1 → first mbed instance = **TWI0 / chip id `i2c0`** |
| E2E marker | `LW_SENSE_OK` (WHO_AM_I ×5) |

## Composition (Layer 2 overlay)

```
arduino-nano-33-ble          (carrier LEDs)
  + ARDUINO_NANO_33_BLE_SENSE_OVERLAY   (Wire1 sensors)
  = arduino-nano-33-ble-sense
```

- **System YAML**: LEDs + 5× `external_device` on **`i2c0`** (Wire1 first-begin → TWI0 @ 0x40003000; not `twi1`)
- **Playground / board-config**: product id `arduino-nano-33-ble-sense`; keep bus id in lockstep with this file
- **E2E**: `examples/arduino-nano-33-ble-sense/verify` → UART `LW_SENSE_OK` (all five WHO_AM_I)

## Sensors (proper declarative models + SimInput)

| type | Addr | SimInput channels |
|------|------|-------------------|
| `lsm9ds1_ag` | 0x6B | `ax` `ay` `az` (g), `gx` `gy` `gz` (°/s) |
| `lsm9ds1_m` | 0x1E | `mx` `my` `mz` (gauss) |
| `hts221` | 0x5F | `humidity` (%), `temperature` (°C) — linear encode |
| `lps22hb` | 0x5C | `pressure` (hPa), `temperature` (°C) |
| `apds9960` | 0x39 | `proximity`, `ambient` (counts); gesture FIFO **not** modelled |

Mic: chip `pdm`. Firmware must raise sensors 3V3 (**P0.22**) and I2C pull-ups
(**P1.00**) before Wire1.

## USB + BLE

Same as [Arduino Nano 33 BLE](arduino-nano-33-ble.md) — `usbd` + `radio` on
the nRF52840 chip model.

## Not this product

Sense **Rev2** (BMI270 / BMM150 / HS3003) needs a separate board id.
