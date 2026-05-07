# Runbook — esp32s3-i2c-tmp102 on real hardware (Plan 4 L4)

Manual procedure to validate the firmware against a real ESP32-S3-Zero
with a physical TMP102 sensor attached. **Not** part of CI.

## Hardware

- ESP32-S3-Zero board (USB ID `303a:1001`)
- TMP102 breakout (e.g. SparkFun, Adafruit, generic AliExpress)
- 4 jumper wires
- USB-C cable

## Wiring

| TMP102 pin | ESP32-S3-Zero pin |
|------------|-------------------|
| SDA        | GPIO8             |
| SCL        | GPIO9             |
| V+         | 3V3               |
| GND        | GND               |
| ADD0       | GND (selects address 0x48) |

The TMP102 is 1.4-3.6 V tolerant, so direct 3.3 V from the dev board is
fine. **Most TMP102 breakouts do NOT include pull-ups.** Add external
4.7 kΩ pull-ups from SDA→3V3 and SCL→3V3 if the breakout doesn't have
them — esp-hal does not enable the GPIO matrix's internal pulls for
I²C lines by default at this version.

## Flash + monitor

From the workspace root with the ESP toolchain on PATH (`source
~/export-esp.sh`):

```
cargo +esp run --release --manifest-path examples/esp32s3-i2c-tmp102/Cargo.toml
```

`cargo run` invokes `espflash flash --monitor` per `.cargo/config.toml`,
flashing the binary and tailing JTAG output.

## Acceptance

The serial monitor should show one line per second:

```
T = 23.50 C   ← real ambient
T = 23.50 C
T = 23.62 C
...
```

If you breathe on the TMP102 you should see the temperature jump up.
Once it crosses 30 °C, GPIO2 goes high (probe with a multimeter or LED
to confirm the level transition).

## Common failures

- **No output at all.** Check `lsusb | grep 303a:1001`. If the board
  is not detected, replug the USB cable. Also check that no other
  process holds the JTAG (`fuser /dev/ttyACM0`).
- **`I2C error: AcknowledgeCheckFailed(Address)` panics in JTAG output.**
  TMP102 not responding. Check wiring; confirm pull-ups are present
  (some breakouts include them, some don't); verify the ADD0 pin is
  tied to a known voltage so the address matches `0x48`.
- **`I2C error: AcknowledgeCheckFailed(Data)`.** Pull-ups too weak or
  bus capacitance too high; try shorter wires or stronger pull-ups
  (2.2 kΩ).
- **All temperatures clamp at -1.0 °C (or read as `0xFFF0`).** TMP102
  ADD0 is floating; tie it to GND.
- **GPIO2 does not toggle.** Threshold is hard-coded at 30.00 °C
  (centi-degrees > 3000). Warm the sensor above that or recompile
  with a lower `THRESHOLD_CENTI_C`.
