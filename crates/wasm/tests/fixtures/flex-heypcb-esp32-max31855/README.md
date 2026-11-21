# HeyPCB ESP32-S3 MAX31855 over a Flex SPI bus

Portable Rust-to-Wasm firmware reads the MAX31855K thermocouple converter of the pinned **HeyPCB ESP32-S3 MAX31855** board, revision A, through a Flex SPI bus. The converter is the catalog `max31855` declarative device model (`configs/devices/max31855.yaml`), the model that the detailed path attaches to an MCU's SPI controller. The firmware drives the IO48 user LED as an over-temperature indicator and prints each reading to a Flex console that stands for the board's native USB serial port. No ESP32-S3 CPU or SPI controller is emulated.

This is the first Flex board on an SPI bus. The generic Flex SPI bus (`spi_buses`, `labwired.spi_transfer`, `spi_transfers`) was added in the same change; no code is specific to this board or this part.

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/max31855), asset `boards/max31855/kicad/esp32_max31855`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs. A regression checks the revision and license against the sibling LDR fixture's source map.
- Bus: the pad nets come from the pinned PCB blob. U4 pad 5 is on `SPI_SCK`, pad 6 on `SPI_CS`, pad 7 on `SPI_MISO`. `SPI_SCK` goes to U3 pad 20 (IO12), `SPI_CS` to U3 pad 18 (IO10), `SPI_MISO` to U3 pad 21 (IO13). The board has no MOSI net. J2 pins 1 and 2 are on `U4_TP` and `U4_TN`, with C8 (10 nF) across them.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

```yaml
spi_buses:
  - name: sensor_spi
    frequency_hz: 1000000
    devices:
      - id: source-u4-max31855
        type: max31855
        cs: 10
        inputs:
          temperature: environment.thermocouple_temperature
          internal: environment.cold_junction_temperature
```

`cs` is the number the firmware passes to `labwired.spi_transfer(bus, cs, tx_ptr, rx_ptr, len) -> i32`; this manifest uses the GPIO number of the `SPI_CS` net. The call returns 0 when a device is declared on `cs` and 1 when none is. With no device the receive buffer is not written: the path does not invent MISO levels.

The firmware ([firmware.rs](firmware.rs), new for this fixture) reads one 4-byte frame in `setup` and one in each `loop`, sends zero bytes (MOSI is not connected), and decodes D31..D18 as a 14-bit signed thermocouple value (0.25 °C per LSB), D15..D4 as a 12-bit signed internal value (0.0625 °C per LSB) and D16 as the fault bit. The indicator goes on at 300.00 °C and off below 280.00 °C. `loop` returns the thermocouple temperature in 0.01 °C plus 100,000, then waits 100 ms.

Assumptions and limits, also carried in `run_report`:

- The firmware is new portable behavior firmware. It is not firmware from the pinned source and not an ESP32-S3 binary.
- The bus speed is 1 MHz. The pinned source states no bus speed; the MAX31855 accepts up to 5 MHz.
- A byte takes eight SCK periods; the device exchanges each byte at its last SCK edge. Firmware instructions take no virtual time.
- The thermocouple input is the temperature the converter reports, not a thermocouple voltage. A read returns the stimulus value at the time of the read; there is no conversion delay.
- Not modeled: SCK, CS and MISO waveforms, SPI mode (CPOL/CPHA), CS setup and hold times, thermocouple faults (the model always reports no fault), K-type nonlinearity and cold-junction compensation error, C8, the 100 ms conversion time, USB enumeration and host input.

## Expected values, calculated before the first run

The model encodes `raw = round(value * scale)` with scale 4 (thermocouple) and 16 (internal), signed and clamped to the field width. 25 °C → 100 → 25.00 °C; 310 / 290 / 279.75 °C → 1240 / 1160 / 1119 → 310.00 / 290.00 / 279.75 °C; -200.25 °C → -801 → -200.25 °C; 1000.1 °C → 4000 → 1000.00 °C. Internal: 22 °C → 352 → 22.0000 °C; -10.0625 °C → -161 → -10.0625 °C.

Time at 1 MHz: a 4-byte frame is 32 SCK periods (32 µs). `setup` = 32 µs. `loop` = 32 µs + 100 ms.

## Measured result

`labwired test`, debug CLI. All three scripts exit 0. Every value above was observed as calculated on the first run.

| Script | Bench stimulus | Firmware result | SPI transfers | LED edges (drive) | Final time |
| --- | --- | --- | --- | --- | --- |
| `test-overtemp-alarm.yaml` | 25 → 310 → 290 → 279.75 °C, CJ 22 °C | 102500, 131000, 129000, 127975 | 5 selected on CS 10, 0 unselected | 100,096,000 and 300,160,000 ns | 400,160,000 ns |
| `test-cold-and-quantized.yaml` | -200.25 then 1000.1 °C, CJ -10.0625 °C | 79975, 200000 | 3 selected on CS 10 | 100,096,000 ns | 200,096,000 ns |
| `test-wrong-chip-select.yaml` (negative control) | 310 °C | `setup` 1, `loop` 0 | 1 unselected on CS 11, 0 selected | none | 32,000 ns |

- Logic analyzer: `user_led_state` follows each drive edge 1,000 ns later. At 290 °C the indicator stays on (hysteresis).
- SPI record (`flex-trace.json`): the second transfer of `test-overtemp-alarm.yaml` is at 32,000 ns for 32,000 ns on CS 10, MOSI `00 00 00 00`, MISO `01 90 16 00` (100 << 18 | 352 << 4).
- Console, first script: `MAX31855 on CS 10 CJ=22.0000 C`, then one line for each sample, from `TC=25.00 C CJ=22.0000 C` to `TC=279.75 C CJ=22.0000 C`. The negative control prints `no MAX31855 on CS 11`.
- In `test-overtemp-alarm.yaml`, moving one expected result and one LED edge time by one unit fails exactly those two assertions (exit 1).

Browser run: see [browser-flex-scripts.json](../browser-flex-scripts.json).

## Onboarding effort

Measured once on 2026-11-20 by the agent that wrote it: 14 min 26 s wall time from the first fetch of the pinned source to three passing native scripts (05:43:59 to 05:58:25 UTC). About 10 min of that was a debug CLI build on a loaded machine. The time includes reading the pinned PCB, BOM and board.json, the firmware, the manifest, the scripts and the calculation of every expected value, with the SHT40 fixture as a template. It excludes the generic Flex SPI bus, which was written before the timer started, and this documentation. No comparable time exists for a chip-specific model of this board, so this number supports no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the SPI controller, thermocouple behavior, or chip parity.
