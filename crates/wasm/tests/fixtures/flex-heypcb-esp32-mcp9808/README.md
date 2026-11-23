# HeyPCB ESP32-S3 MCP9808 over a Flex I2C bus

Portable Rust-to-Wasm firmware reads the MCP9808 temperature sensor of the pinned **HeyPCB ESP32-S3 MCP9808** board, revision A, through a Flex I2C bus. The sensor is the catalog `mcp9808` declarative device model (`configs/devices/mcp9808.yaml`), the model that the detailed path attaches to an MCU's I2C controller. The firmware drives the IO48 user LED as a frost indicator and prints each reading to a Flex console that stands for the board's native USB serial port. No ESP32-S3 CPU or I2C controller is emulated.

This fixture adds no Flex code that is specific to it. It uses the generic Flex I2C bus, console and behavior LED of the TMP117 fixture. It is the first Flex board on a catalog model with seeded noise and thermal lag. It found two defects in the catalog model, fixed in the same change (see [Model fix](#model-fix)).

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/mcp9808), asset `boards/mcp9808/kicad/esp32_mcp9808`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs.
- Bus: the pad nets come from the pinned PCB blob. U4 (MCP9808T-E/MS) pad 1 is on `I2C_SDA`, pad 2 on `I2C_SCL`, pad 3 (ALERT) on `SNS_INT`, pads 5, 6 and 7 (A2, A1, A0) on `GND`. A2:A0 on GND gives address 0x18; the source README states the same address. `I2C_SCL` goes to U3 pad 17 (IO9), `I2C_SDA` to U3 pad 12 (IO8), `SNS_INT` to U3 pad 7 (IO7) with the R6 10 kΩ pull-up. R4 connects IO48 to D1.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

```yaml
i2c_buses:
  - name: sensor_i2c
    frequency_hz: 100000
    devices:
      - id: source-u4-mcp9808
        type: mcp9808
        config: { i2c_address: 0x18, noise_sigma: 0 }
        inputs:
          temperature: environment.temperature
```

The firmware ([firmware.rs](firmware.rs), new for this fixture) reads the manufacturer ID (pointer 0x06, 0x0054) and device ID (0x07, high byte 0x04). It writes -5.00 °C to T_LOWER and -4.50 °C to T_UPPER and reads T_LOWER back. It writes RESOLUTION (0x08) = 0.0625 °C and reads it back. Each `loop` reads T_A (0x05), takes bits 12:0 as a 13-bit two's-complement count of 0.0625 °C, and compares the count with the two limits. The indicator goes on below the low limit and off above the high limit. `loop` returns the temperature in 1/10,000 °C plus 1,000,000, then waits 250 ms, the conversion time at 0.0625 °C.

The placement sets `noise_sigma: 0`, so the scripts can assert exact values. The catalog thermal lag (first order, time constant 1 s) stays on. The model updates it at each register read, from the device time on the Flex bus.

Assumptions and limits, also carried in `run_report`:

- The firmware is new portable behavior firmware. It is not firmware from the pinned source and not an ESP32-S3 binary.
- The bus speed is 100 kHz. The pinned source states no bus speed.
- A transfer takes one SCL period for each bit, START and STOP. Firmware instructions take no virtual time.
- Not modeled: the ALERT output and R6, the T_A comparator flags (bits 15:13), SCL/SDA waveforms, the R7 and R8 pull-ups, clock stretching, conversion time and the coarser LSB of lower resolutions, shutdown, USB enumeration and host input, sensor accuracy and reading noise.

## Expected values, calculated before the first run

A small calculator replayed the read times through the model's lag update (`out += (1 - exp(-dt / 1 s)) * (x - out)`, the first read starts at the stimulus) and the encoding `round(°C × 16)` as 13-bit two's complement.

Time at 100 kHz: a word read (pointer write, repeated START, 2 bytes) is 48 bits = 480 µs; a byte read 39 bits = 390 µs; a limit write 38 bits = 380 µs; the RESOLUTION write 29 bits = 290 µs. `setup` = 480 + 480 + 380 + 380 + 480 + 290 + 390 µs = 2.88 ms. `loop` = 480 µs + 250 ms.

- `test-frost-alarm.yaml`: 25 °C, then 16 conversions at -10.125 °C (the lag reaches -9.5000 °C), then 16 at 0 °C. The indicator goes on at -5.3750 °C (count -86 < -80), stays on at -4.5000 °C (count -72, the high limit), and goes off at -3.5000 °C.
- `test-sub-zero-range.yaml`: -40 °C from before `setup` (count -640, word 0x1D80), then 40 conversions at -0.0625 °C (count -1, word 0x1FFF) and 40 at 125 °C (count 2000).
- `test-wrong-address.yaml`: negative control at 0x19 (A0 on VDD); no device answers.

## Measured result

`labwired test`, debug CLI. All three scripts exit 0 on the first run. Every value above was observed as calculated.

| Script | Bench stimulus | Firmware result | I2C transfers | LED edges (drive) | Final time |
| --- | --- | --- | --- | --- | --- |
| `test-frost-alarm.yaml` (11 assertions) | 25 → -10.125 → 0 °C | 1250000, 905000 (16th at -10.125 °C), 998125 | 40 acknowledged at 0x18 | 2,007,200,000 and 5,012,960,000 ns | 8,268,720,000 ns |
| `test-sub-zero-range.yaml` (10 assertions) | -40, -0.0625, 125 °C | 600000, 999375, 2250000 | 88 acknowledged at 0x18 | 3,360,000 and 2,257,680,000 ns | 20,291,760,000 ns |
| `test-wrong-address.yaml` (8 assertions, negative control) | -20 °C | `setup` 1, `loop` 0 | 1 refused at 0x19, 0 acknowledged | none | 110,000 ns |

- Logic analyzer: `user_led_state` follows each drive edge 1,000 ns later.
- Console, first script: `MCP9808 0x0054/0x0400 at 0x18 frost below -5.0000 C`, then 33 readings from `T=25.0000 C` through `T=-9.5000 C` (lag toward -10.125 °C) and back to `T=-0.1875 C`.
- The first script runs 8.27 s of virtual time in 1.58 s wall time; the second 20.29 s in 5.03 s (debug CLI).

Negative controls, on `/tmp` copies:

- Pre-fix model: the same manifest plus the pre-fix `mcp9808.yaml` as a manifest part pack (`overrides: mcp9808`). `setup` returns 4 and prints `resolution reads 0x01`: pointer 0x08 aliased to pointer 0x00, which held T_A. Every sub-zero stimulus is refused: `value -10.125 for channel 'temperature' is out of range [0, 125]`. The scripts fail 10 of 11, 10 of 10 and 1 of 8 assertions.
- Moved values: one expected result and one time moved by one unit in each script fail exactly those two assertions (exit 1).

Browser run: see [browser-flex-scripts.json](../browser-flex-scripts.json).

## Model fix

The held-out firmware follows the MCP9808 datasheet (DS20005095B). Before the fix, the catalog model disagreed with it in two places:

1. Register map. The model put T_A at pointer 0x00 and RESOLUTION at 0x05, with a 3-bit pointer mask. Table 5-1 puts T_A at 0x05, RESOLUTION at 0x08, and 0x00 is reserved. A datasheet driver that reads 0x05 got the resolution byte, not the temperature.
2. Sign. T_A clamped at 0 °C. Register 5-4 has a sign bit at bit 12, so -10.125 °C is 0x1F5E.

The fix changes only `configs/devices/mcp9808.yaml`: the datasheet pointers with a 4-bit mask, T_A as a signed 13-bit `fields:` entry, the datasheet write masks on CONFIG, the limit registers and RESOLUTION, the input range -40 to 125 °C, and the existing generic `noise_sigma_key` so that a placement can set the noise. No engine code changed. The core tests `mcp9808_ta_is_pointer_5_and_signed_13_bit` and `mcp9808_limit_registers_drop_unimplemented_bits` cover it.

## Onboarding effort

Measured once on 2026-11-23 by the agent that wrote it: 22 min 28 s wall time from the first fetch of the pinned source (17:59:43 UTC, file time) to three passing native scripts (18:22:11 UTC). That includes reading the catalog model and the datasheet register map, the model fix, the firmware, the manifest, the scripts, the calculator for every expected value, and one debug CLI build of 15 min 29 s on a shared machine. It excludes this documentation, the negative controls and the browser run. No comparable time exists for a chip-specific model of this board, so this number supports no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the I2C controller, the ALERT pin, sensor accuracy, or chip parity.
