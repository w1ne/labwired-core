# HeyPCB ESP32-S3 SHT40 over a Flex I2C bus

Portable Rust-to-Wasm firmware reads the SHT40 of the pinned **HeyPCB ESP32-S3 SHT40** board, revision A, through a Flex I2C bus. The sensor is the catalog `sht40` declarative device model (`configs/devices/sht40.yaml`), the model that the detailed path attaches to an MCU's I2C controller. The firmware drives the IO48 user LED as a high-humidity indicator and prints each reading to a Flex console that stands for the board's native USB serial port. No ESP32-S3 CPU or I2C controller is emulated.

This board was held out: the Flex runtime, the manifest grammar and the catalog model were not changed to make it run. The run found two generic timing defects, which are fixed in shared code (see "Defects found").

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/sht40), asset `boards/sht40/kicad/esp32_sht40`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs. A regression checks the revision and license against the sibling LDR fixture's source map.
- Bus: the pad nets come from the pinned PCB blob. U4 pad 1 is on `I2C_SDA`, pad 2 on `I2C_SCL`, pad 3 on 3V3 and pad 4 on GND. `I2C_SDA` goes to U3 pad 12 (IO8) and `I2C_SCL` to U3 pad 17 (IO9). R6 and R7 (4.7 kΩ) pull both lines to 3V3.
- Address: the pinned BOM fits `SHT40-AD1B-R2`. The SHT4x has no address pin; the `-AD1B` order code is address 0x44, as the source README states.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

The Library lane runs ESP32-S3 test firmware for the same asset (`heypcb:HeyPCB/esp32-sensors-1:boards/sht40/kicad/esp32_sht40`) on the detailed path. This fixture does not replace that run and proves nothing about it.

## Model

```yaml
i2c_buses:
  - name: sensor_i2c
    frequency_hz: 100000
    devices:
      - id: source-u4-sht40
        type: sht40
        config: { i2c_address: 0x44 }
        inputs:
          temperature: environment.temperature
          humidity: environment.humidity
```

The firmware ([firmware.rs](firmware.rs), 254 lines, new for this fixture) calls `labwired.i2c_transfer`. `setup` sends a soft reset (0x94), waits 1 ms, sends the serial-number command (0x89), waits 1 ms, and reads two CRC-framed words. `loop` sends the high-precision measurement command (0xFD), waits the time that `setup` received, reads six bytes, and rejects a frame whose Sensirion CRC-8 does not match. It converts with `T = -45 + 175 * S_T / 65535` and `RH = -6 + 125 * S_RH / 65535` (SHT4x datasheet), in hundredths, rounded to nearest. The indicator goes on at 70.00 %RH and off below 65.00 %RH. `loop` returns the humidity in 0.01 %RH plus 10,000.

Assumptions and limits, also carried in `run_report`:

- The firmware is new portable behavior firmware. It is not firmware from the pinned source and not an ESP32-S3 binary.
- The bus speed is 100 kHz. The pinned source states no bus speed.
- A transfer takes one SCL period for each bit, START and STOP. Firmware instructions take no virtual time.
- A measurement answers after the datasheet maximum (8.3 ms at high precision) with the stimulus value at the time of the command.
- Deviation: the part NACKs a read header before its measurement is complete. The model acknowledges and answers `0xFF` bytes. The firmware detects them by CRC.
- Not modeled: SCL and SDA waveforms, R6 and R7, bus capacitance, clock stretching, heater effect, repeatability noise, response time, soft-reset and power-up times, USB enumeration and host input.

## Expected values, calculated before the first run

The model encodes `raw = round(value * scale + offset)` with the scale and offset of `sht40.yaml`; the firmware inverts that with rounding to nearest. 25 °C → 26214 → 25.00 °C; -10 °C → 13107 → -10.00 °C; 50/60/68/72/90 %RH → 29360/34602/38797/40894/50331 → 50.00/60.00/68.00/72.00/90.00 %RH. The serial number is the model constant `0x0f4e2a71`.

Time at 100 kHz: a one-byte command is 20 bit times (200 µs), a six-byte read 65 bit times (650 µs), a refused address 11 bit times (110 µs). `setup` = 200 + 1000 + 200 + 1000 + 650 µs = 3.05 ms. `loop` = 200 µs + wait + 650 µs + 1 ms period; with a 10 ms wait, 11.85 ms.

## Measured result

`labwired test`, debug CLI. All four scripts exit 0. Every value above was observed as calculated on the first run.

| Script | Bench stimulus | Firmware result | I2C transfers | LED edges (drive) | Final time |
| --- | --- | --- | --- | --- | --- |
| `test-humidity-alarm.yaml` | 50 → 72 → 72 → 68 → 60 %RH, 25 °C | 15000, 17200, 17200, 16800, 16000 | 13 acknowledged at 0x44, 0 refused | 25,750,000 and 61,300,000 ns | 62,300,000 ns |
| `test-cold-and-humid.yaml` | -10 °C, 90 %RH | 19000; temperature 9000 (-10.00 °C) | 5 acknowledged | 13,900,000 ns | 14,900,000 ns |
| `test-wrong-address.yaml` (negative control) | 72 %RH | `setup` 1, `loop` 0 | 1 refused at 0x45, 0 acknowledged | none | 110,000 ns |
| `test-read-too-early.yaml` (negative control) | 72 %RH | `loop` 0 (CRC error) | 7 acknowledged | none | 8,750,000 ns |

- Logic analyzer: `user_led_state` follows each drive edge 1,000 ns later. At 68 %RH the indicator stays on (hysteresis).
- Console, first script: `SHT40 serial 0x0f4e2a71 at 0x44`, then one line for each sample, from `T=25.00 C RH=50.00 %` to `T=25.00 C RH=60.00 %`. The negative controls print `no SHT40 at 0x45` and `SHT40 CRC error`.
- In `test-humidity-alarm.yaml`, moving one expected result or one D1 edge time by one unit fails exactly that assertion. With a 8,300 µs wait, `test-read-too-early.yaml` fails on five assertions.

Browser run ([browser-flex-scripts.json](../browser-flex-scripts.json)): Chromium 156.0.8078.79 with Playwright ran the four scripts through `WasmFlexTestRun` in the web package built from Core commit `b1b0b5d26`. Each result text equals the `labwired test` result text, and the logic-analyzer capture, the I2C transfer record and the console record are equal. The script runs took 12 ms, 22 ms, 6 ms, 3 ms in the page (test-cold-and-humid.yaml, test-humidity-alarm.yaml, test-read-too-early.yaml, test-wrong-address.yaml; dev build, one run each). The result hashes of the 35 older scripts did not change with the two timing fixes.

## Defects found

Both are generic and affect every command device with `delay_us` (SHT3x, SHT4x and others), not this board only.

1. **The Flex bus gave every byte to the device at the START.** The device clock advanced only after the transfer, so a measurement started up to one transfer early. A read 8,100 µs after the end of the command returned data; the datasheet maximum is 8,300 µs. `I2cBuses::transfer` now gives a written byte to the device after its ACK bit, samples a read byte at its first bit and sends STOP at the end, and runs the device clocks between those events. The time the clocks ran inside a transfer is not counted again by the following `advance_ns`.
2. **A read that started too early switched to data in the middle.** With bytes at their bit times, a six-byte read that started 1 µs before the response was ready returned `0xFF` and then the first data bytes. The declarative command device now decides readiness at the first byte of a read phase; the part NACKs the read header, so such a transfer has no data. This also applies on the detailed path.

Regression: `flex_i2c::tests::a_command_delay_counts_from_the_command_byte_not_the_start`. The command byte is complete at 190 µs, the first read byte is sampled 100 µs after its START, so a wait of 8,189 µs after the command transfer gives six `0xFF` bytes and 8,190 µs gives data. Before the fixes the threshold was 8,100 µs.

## Onboarding effort

Measured once on 2026-11-20 by the agent that wrote it: 3 min 49 s wall time from the start of the work on this board to four passing native scripts (04:31:12 to 04:35:01 UTC). This includes reading the pinned source files, the firmware, the manifest, the scripts and the calculation of every expected value. It reused the BME280 fixture as a template and the existing catalog `sht40` model. It excludes the two defect fixes and this documentation. No new Rust model code, runtime code or tool was written for the board. No comparable time exists for a chip-specific model of this board, so this number supports no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the I2C controller, the board's bus wiring under electrical load, or chip parity.
