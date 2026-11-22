# HeyPCB ESP32-S3 TMP117 over a Flex I2C bus

Portable Rust-to-Wasm firmware reads the TMP117 temperature sensor of the pinned **HeyPCB ESP32-S3 TMP117** board, revision A, through a Flex I2C bus. The sensor is the catalog `tmp117` declarative device model (`configs/devices/tmp117.yaml`), the model that the detailed path attaches to an MCU's I2C controller. The firmware drives the IO48 user LED as an over-temperature indicator and prints each reading to a Flex console that stands for the board's native USB serial port. No ESP32-S3 CPU or I2C controller is emulated.

This fixture adds no Flex code that is specific to it. It uses the generic Flex I2C bus, console and behavior LED of the SHT40 and BME280 fixtures. The fixture found one generic defect, fixed in the same change: a firmware wait longer than 0.1 s failed (see [Runtime fix](#runtime-fix)).

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a19ac1aa9c092e43fb5b123c461508752`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/tmp117), asset `boards/tmp117/kicad/esp32_tmp117`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs. A regression checks the revision and license against the sibling LDR fixture's source map.
- Bus: the pad nets come from the pinned PCB blob. U4 (TMP117AIDRVR) pad 1 is on `I2C_SCL`, pad 6 on `I2C_SDA`, pad 3 (ALERT) on `SNS_INT`, pad 4 (ADD0) on `GND`. ADD0 on GND gives address 0x48; the source README states the same address. `I2C_SCL` goes to U3 pad 17 (IO9), `I2C_SDA` to U3 pad 12 (IO8), `SNS_INT` to U3 pad 7 (IO7) with the R6 10 kΩ pull-up.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

```yaml
i2c_buses:
  - name: sensor_i2c
    frequency_hz: 100000
    devices:
      - id: source-u4-tmp117
        type: tmp117
        config: { i2c_address: 0x48 }
        inputs:
          temperature: environment.temperature
```

The firmware ([firmware.rs](firmware.rs), new for this fixture) reads DEVICE_ID (0x0F) and checks device number 0x117. It writes 30.00 °C to T_HIGH_LIMIT and 29.50 °C to T_LOW_LIMIT and reads T_HIGH_LIMIT back. Each `loop` reads CONFIGURATION, requires DATA_READY (D13), reads TEMP_RESULT (int16, 7.8125 m°C per LSB), and compares the result with the two limits. The indicator goes on at the high limit and off below the low limit. `loop` returns the temperature in m°C (rounded half away from zero) plus 100,000, then waits 1 s, the conversion cycle of the power-on configuration.

The model has no alert comparator: the limits are storage, and the ALERT pin never changes. The firmware therefore does the comparison itself. A real TMP117 driver can use the ALERT pin on IO7 instead; that path is not available, and the manifest lists it in `unsupported_checks`.

Assumptions and limits, also carried in `run_report`:

- The firmware is new portable behavior firmware. It is not firmware from the pinned source and not an ESP32-S3 binary.
- The bus speed is 100 kHz. The pinned source states no bus speed.
- A transfer takes one SCL period for each bit, START and STOP. Firmware instructions take no virtual time.
- TEMP_RESULT gives the stimulus temperature at the time of the read. DATA_READY always reads set.
- Not modeled: the ALERT output and R6, SCL/SDA waveforms, the R7 and R8 pull-ups, clock stretching, the conversion and averaging time, the TEMP_OFFSET addition, EEPROM programming, USB enumeration and host input.

## Expected values, calculated before the first run

The model encodes `raw = round(°C × 128)` as int16. 25 → 3200 → 25.000 °C. 30.1 / 29.6 / 29.4 °C → 3853 / 3789 / 3763 → 30.102 / 29.602 / 29.398 °C. -40.0078125 °C → -5121 → -40.008 °C. 0.004 °C → 1 → 0.008 °C. 150 °C → 19200 → 150.000 °C. The limits are 3840 (30.00 °C) and 3776 (29.50 °C).

Time at 100 kHz: a register read (pointer write, repeated START, 2 bytes) is 48 bits = 480 µs; a limit write (pointer and 2 bytes) is 38 bits = 380 µs. `setup` = 480 + 380 + 380 + 480 µs = 1.72 ms. `loop` = 2 × 480 µs + 1 s.

## Measured result

`labwired test`, debug CLI. All three scripts exit 0. Every value above was observed as calculated, after the runtime fix below.

| Script | Bench stimulus | Firmware result | I2C transfers | LED edges (drive) | Final time |
| --- | --- | --- | --- | --- | --- |
| `test-overtemp-alarm.yaml` | 25 → 30.1 → 29.6 → 29.4 °C | 125000, 130102, 129602, 129398 | 12 acknowledged at 0x48 | 1,003,640,000 and 3,005,560,000 ns | 4,005,560,000 ns |
| `test-resolution-and-range.yaml` | -40.0078125, 0.004, 150 °C | 59992, 100008, 250000 | 10 acknowledged at 0x48 | 2,004,600,000 ns | 3,004,600,000 ns |
| `test-wrong-address.yaml` (negative control) | 40 °C | `setup` 1, `loop` 0 | 1 refused at 0x49, 0 acknowledged | none | 110,000 ns |

- Logic analyzer: `user_led_state` follows each drive edge 1,000 ns later. At 29.6 °C the indicator stays on (hysteresis).
- I2C record (`flex-trace.json`), first script: DEVICE_ID read at 0 ns, write `0f`, read `01 17`; limit writes `02 0f 00` and `03 0e c0`; read-back `0f 00`; CONFIGURATION read `22 20`; TEMP_RESULT read `0c 80` (3200).
- Console, first script: `TMP117 ID 0x0117 at 0x48 high 30.000 C`, then `T=25.000 C`, `T=30.102 C`, `T=29.602 C`, `T=29.398 C`. The negative control prints `no TMP117 at 0x49`.
- In `test-overtemp-alarm.yaml`, moving one expected result and one LED edge time by one unit fails exactly those two assertions (exit 1).
- The first script runs 4.0 s of virtual time in 0.71 s wall time (debug CLI).

Browser run: see [browser-flex-scripts.json](../browser-flex-scripts.json).

## Runtime fix

Before the fix, `test-overtemp-alarm.yaml` stopped in its first `loop` with `stop_reason=ModelError`: `Flex advance requests 1000000 simulation steps (including analog substeps); maximum per call is 100000`. The 1 s wait crosses 1,000,000 boundaries of the 1 µs LED behavior model, and one `FlexRuntime::advance_ns` call allows 100,000 model steps. Any portable firmware with a `delay(1000)` and a 1 µs model hit the same limit.

The script runner now advances a firmware wait with `FlexRuntime::advance_ns_in_pieces`. Each piece stays in the per-call budget; the whole run stays bounded by `limits.max_time_ns` and the fuel limit. A direct `advance_ns` call from JavaScript keeps the per-call limit. The core test `advance_in_pieces_covers_a_wait_over_the_call_budget` checks that a 1 s wait in pieces gives the same digital trace as 1,000 calls of 1 ms.

## Onboarding effort

Measured once on 2026-11-20 by the agent that wrote it: 20 min 51 s wall time from the first fetch of the catalog model and the pinned source to three passing native scripts (08:18:28 to 08:39:19 UTC). That includes two debug CLI builds (5 min 58 s after a merge of `main`, and 3 min 3 s after the runtime fix) and the runtime fix itself. It includes reading the pinned PCB, BOM, README and board.json, the firmware, the manifest, the scripts and the calculation of every expected value, with the SHT40 fixture as a template. It excludes this documentation and the browser run. No comparable time exists for a chip-specific model of this board, so this number supports no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the I2C controller, the ALERT pin, sensor accuracy, or chip parity.
