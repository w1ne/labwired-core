# Demo Blinky — GPIO + Virtual I²C Sensor (TMP102)

> Part of the [LabWired Demos](../../../DEMOS.md) suite.

Canonical example for **attaching an external device model to an on-chip
bus** in LabWired. Firmware runs on a simulated STM32F103 and talks to a
virtual TMP102 temperature sensor over I²C1, while also blinking an LED on
GPIOA.

This is the example to copy when you need to wire any new external I²C/SPI
device (sensor, EEPROM, IMU, etc.) into a board model.

## What it demonstrates

1. **External device attach pattern.** A TMP102 model is bound to the I²C1
   peripheral at address `0x48` via `external_devices` in `system.yaml`.
2. **I²C transaction round-trip.** Firmware issues real I²C reads against
   the modeled bus; the sensor model responds.
3. **Board I/O mapping.** GPIO LED on PA5 is exposed via `board_io` for
   simulator-side visibility / assertions.

## System manifest — the pattern to copy

```yaml
# examples/demo-blinky/system.yaml
name: "demo-blinky-board"
chip: "../../configs/chips/stm32f103.yaml"
external_devices:
  - id: "temp_sensor"
    type: "tmp102"
    connection: "i2c1"            # must match a peripheral id in the chip yaml
    config:
      descriptor: "../../configs/peripherals/tmp102.yaml"
      i2c_address: 0x48
board_io:
  - id: "led_pa5"
    kind: "led"
    peripheral: "gpioa"
    pin: 5
    signal: "output"
    active_high: true
```

The three things to know:

- `connection:` references a peripheral `id` from the chip yaml. The bus
  must exist in `configs/chips/<chip>.yaml` and be modeled — see
  [`docs/boards/`](../../docs/boards/) for per-chip coverage.
- `type:` selects which built-in device model to use. New device types
  live in `crates/labwired-devices/`; YAML-defined declarative devices
  reference a `descriptor:` path.
- `i2c_address:` is the 7-bit bus address; the model handles all
  read/write framing.

## Building

```bash
cd examples/demo-blinky
make
```

## Running in LabWired

```bash
cargo run -p labwired-cli -- \
  --firmware examples/demo-blinky/build/demo-blinky.elf \
  --system examples/demo-blinky/system.yaml
```

## Expected Output

```
GPIO: Write to GPIOA_ODR: 0x00000020 (LED ON)
I2C1 -> tmp102@0x48: read TEMP register -> 0x1900 (25.0°C)
GPIO: Write to GPIOA_ODR: 0x00000000 (LED OFF)
...
```

## Adapting to a different chip

The pattern is chip-agnostic, but the bus has to be modeled for your
target. Before copying this manifest to a new chip:

1. Check `configs/chips/<your_chip>.yaml` — is there an `i2c*` peripheral
   listed?
2. Check [`docs/boards/<your_chip>.md`](../../docs/boards/) — is that bus
   marked modeled vs. yaml-only?

If the bus isn't modeled, the firmware will compile but will hit
`MemoryAccessViolation` or a stuck polling loop the first time it
touches the I²C registers. See
[`docs/getting_started_firmware.md`](../../docs/getting_started_firmware.md#4-common-runtime-issues).

For Xtensa / ESP32-S3 see
[`examples/esp32s3-i2c-tmp102/`](../esp32s3-i2c-tmp102/) — same TMP102
device, attached to I²C0 on the `esp32s3-zero` chip variant.
