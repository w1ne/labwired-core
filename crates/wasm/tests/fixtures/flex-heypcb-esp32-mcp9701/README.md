# HeyPCB ESP32-S3 MCP9701A on a Flex analog line

Portable Rust-to-Wasm firmware reads the MCP9701A analog temperature sensor of the pinned **HeyPCB ESP32-S3 MCP9701A** board, revision A. The sensor is the catalog `mcp9701` analog-source descriptor (`configs/devices/mcp9701.yaml`), which another lane added for the detailed ESP32 path (w1ne/labwired-core#2110). Once a second the firmware reads the sensor output in millivolts on IO1, converts it to a temperature, prints it to a Flex console that stands for the board's native USB serial port, and drives the IO48 user LED as an over-temperature indicator. No ESP32-S3 CPU or ADC peripheral is emulated.

This board is a held-out reuse check. The Flex lane did not write its part model, and the board needed no Rust model code, no new manifest section, tool, importer, MCP command or board-specific code. It runs through the existing `behavior` adapter, `labwired test`, logic analyzer, oscilloscope and console.

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/mcp9701), asset `boards/mcp9701/kicad/esp32_mcp9701`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs; each one was checked with `git hash-object` on the downloaded file. A regression checks the revision and license against the sibling LDR fixture's source map.
- Line: the pad nets come from the pinned PCB blob. U4 (MCP9701A-E/TO, TO-92, LCSC C73090) pin 2 is on `SNS_AN1`, pin 1 on `3V3`, pin 3 on `GND`. This agrees with data sheet Table 3-1 for TO-92 (1 VDD, 2 VOUT, 3 GND). `SNS_AN1` goes to U3 pad 39 (IO1). The README states "400 mV at 0 C, 19.5 mV/C, read on IO1; 100 nF on VDD".
- Part: Microchip MCP970X family data sheet DS20001942L, sha256 `5b0e0efe…7aff33`, the same file as the MCP9700 fixture (full value in the source map). Page 3, MCP9701/9701A: V_0°C = 400 mV, T_C = 19.5 mV/°C, VDD 3.1..5.5 V, TA -10..+125 °C; TO-92 thermal response 1.65 s.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

```yaml
cosim_models:
  - id: source-u4-mcp9701a
    adapter: behavior
    step_ns: 1000000
    inputs: { temperature: environment.temperature }
    outputs: { voltage_mv: source.net.SNS_AN1 }
    config: { device_type: mcp9701 }
```

The descriptor formula is `min(max(400 + 19.5 * temperature, 0), 2837.5)` with `encode: trunc`, so the output is whole millivolts. A temperature outside -10..125 °C is refused. The model is evaluated every 1 ms.

The firmware ([firmware.rs](firmware.rs)) is the MCP9700 fixture firmware with two changes: the transfer constants (400 mV, 19.5 mV/°C) and the conversion line, T = (mV − 400) × 100 / 195 tenths of °C with integer truncation. The thresholds (on at 40.0 °C, off below 38.0 °C), the console text and the `loop` return value are the same. `loop` returns (10,000 + T in tenths) × 10 + LED.

Assumptions and limits, also carried in `run_report`:

- The firmware is new portable behavior firmware. It is not firmware from the pinned source and not an ESP32-S3 binary.
- The millivolt value is exact. ESP32-S3 ADC quantization, attenuation, calibration error and noise are not modeled.
- Not modeled: MCP9701A accuracy, nonlinearity, line regulation, output impedance, turn-on time, thermal response, C7, USB enumeration and host input.

## Expected values, calculated before the first run

A test-bench `set` at k s reaches the output at the next 1 ms boundary, k s + 1,000,000 ns, where the firmware samples it. An LED write reaches `user_led_state` 1,000 ns later. An even temperature gives whole millivolts, so the firmware recovers it exactly; other values lose up to 0.1 °C to truncation.

| Script | Ambient (°C) | V_out (mV) | `loop` results | LED drive edges (ns) |
| --- | --- | --- | --- | --- |
| `test-overtemp-alarm.yaml` | 26, 42, 38, 36 | 907, 1219, 1141, 1102 | 102600, 104201, 103801, 103600 | 1,001,000,000 (on), 3,001,000,000 (off) |
| `test-range-ends.yaml` | -10, 0, 125, 25.37 | 205, 400, 2837, 894 | 99000, 100000, 112491, 102530 | 2,001,000,000 (on), 3,001,000,000 (off) |
| `test-below-threshold.yaml` (negative control) | 26, 36, 39.95 | 907, 1102, 1179 | 102600, 103600, 103990 | none |

## Measured result

`labwired test`, debug CLI, 2026-11-21. All three scripts exit 0 on their first run with 16, 12 and 9 assertions. Every value above was observed as calculated.

- Oscilloscope: `sensor_mv` is 887 mV at 0 ns (the 25 °C port default), crosses 1180 mV rising at 1,001,000,000 ns and 1120 mV falling at 3,001,000,000 ns in the alarm script. The runs keep all 4,001, 4,001 and 3,001 oscilloscope samples (none dropped). In the negative control its maximum is 1179 mV and it has no 1180 mV crossing.
- Logic analyzer: `user_led_drive` and `user_led_state` edges at the times above; hysteresis keeps the indicator on at 38 °C.
- Console, alarm script: `T=26.0 C`, `T=42.0 C ALARM`, `T=38.0 C ALARM`, `T=36.0 C`. Range script: `T=-10.0 C`, `T=0.0 C`, `T=124.9 C ALARM`, `T=25.3 C`.
- Wall time, debug CLI, single runs: 1.20 s, 1.10 s and 0.85 s for 4, 4 and 3 s of virtual time.

Negative controls (`/tmp` copies, not committed):

- 40.00 °C instead of 39.95 °C in the below-threshold script: 6 of 9 assertions fail. `loop` returns 104001, `sensor_mv` reaches 1180 mV and crosses 1180 mV at 2,001,000,000 ns, and the LED drive edge appears at 2,001,000,000 ns.
- -11 °C instead of -10 °C in the range script: exit 3, `stop_reason: model_error`, message `analog-source input 'temperature': value -11 for channel 'temperature' is out of range [-10, 125]`; 11 of 12 assertions fail.
- The wrong part, `device_type: mcp9700`, on this board: 13 of 16 alarm-script assertions fail. `sensor_mv` is 750 mV at 0 ns and peaks at 920 mV, so the indicator never turns on.

Browser run: see [browser-flex-scripts.json](../browser-flex-scripts.json).

## Onboarding effort

Measured once on 2026-11-21, from command times in one lane session. About 21 min wall time from the first listing of the pinned board folder (03:05 UTC) to three passing scripts (03:26 UTC). Source download and blob check, data sheet checks, firmware, manifest, scripts and source map were written while the peripherals manifest regeneration and the debug CLI build after the merge of `origin/main` ran (about 18 min together, load average about 35, shared with other lanes). The part model already existed. No comparable time exists for a chip-specific model of this board, so these numbers support no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the ADC peripheral, sensor accuracy, or chip parity.
