# HeyPCB ESP32-S3 MCP9700 on a Flex analog line

Portable Rust-to-Wasm firmware reads the MCP9700 analog temperature sensor of the pinned **HeyPCB ESP32-S3 MCP9700** board, revision A. The sensor is the new catalog `mcp9700` analog-source descriptor (`configs/devices/mcp9700.yaml`), built from the Microchip data sheet. Once a second the firmware reads the sensor output in millivolts on IO1, converts it to a temperature, prints it to a Flex console that stands for the board's native USB serial port, and drives the IO48 user LED as an over-temperature indicator. No ESP32-S3 CPU or ADC peripheral is emulated.

The board needed no Rust model code. The part is one data-only descriptor on the existing `analog_source` primitive; the fixture runs through the existing `behavior` adapter, `labwired test`, logic analyzer, oscilloscope and console. There is no new manifest section, tool, importer, MCP command or board-specific code.

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/mcp9700), asset `boards/mcp9700/kicad/esp32_mcp9700`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs. A regression checks the revision and license against the sibling LDR fixture's source map.
- Line: the pad nets come from the pinned PCB blob. U4 (MCP9700T-E/LT, SC70-5, LCSC C511278) pad 3 is on `SNS_AN1`, pad 4 on `3V3`, pad 2 on `GND`, pads 1 and 5 not connected. This agrees with data sheet Table 3-1 (2 GND, 3 VOUT, 4 VDD). `SNS_AN1` goes to U3 pad 39 (IO1). The README states "500 mV at 0 C, 10 mV/C, read on IO1 (ADC1 channel 0); 100 nF on VDD".
- Part: Microchip MCP970X family data sheet DS20001942L, sha256 `5b0e0efe…7aff33` (full value in the source map). Equation 4-1, V_out = T_C × T_A + V_0°C, with V_0°C = 500 mV and T_C = 10.0 mV/°C; extended range -40..+125 °C, output 100 mV to 1.75 V.
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

```yaml
cosim_models:
  - id: source-u4-mcp9700
    adapter: behavior
    step_ns: 1000000
    inputs: { temperature: environment.temperature }
    outputs: { voltage_mv: source.net.SNS_AN1 }
    config: { device_type: mcp9700 }
```

The descriptor formula is `min(max(500 + 10 * temperature, 0), 1750)` with `encode: trunc`, so the output is whole millivolts. A temperature outside -40..125 °C is refused; the +150 °C grade is another order code. The model is evaluated every 1 ms, about 1,300 times faster than the 1.3 s SC70 thermal response, which is not modeled.

The firmware ([firmware.rs](firmware.rs), new for this fixture) samples 1 ms into each 1 s period. It reads whole millivolts, as Arduino-ESP32 `analogReadMilliVolts()` returns them, and computes T = (mV − 500) / 10 °C in tenths. The indicator goes on at 40.0 °C (900 mV) and off below 38.0 °C (880 mV). It prints `T=<°C> C`, with ` ALARM` while the indicator is on. `loop` returns (10,000 + T in tenths) × 10 + LED.

Assumptions and limits, also carried in `run_report`:

- The firmware is new portable behavior firmware. It is not firmware from the pinned source and not an ESP32-S3 binary.
- The millivolt value is exact. ESP32-S3 ADC quantization, attenuation, calibration error and noise are not modeled.
- Not modeled: MCP9700 accuracy, nonlinearity, line regulation, output impedance, turn-on time, thermal response, C7, USB enumeration and host input.

## Expected values, calculated before the first run

A test-bench `set` at k s reaches the output at the next 1 ms boundary, k s + 1,000,000 ns, where the firmware samples it. An LED write reaches `user_led_state` 1,000 ns later.

| Script | Ambient (°C) | V_out (mV) | `loop` results | LED drive edges (ns) |
| --- | --- | --- | --- | --- |
| `test-overtemp-alarm.yaml` | 25, 41, 39, 37 | 750, 910, 890, 870 | 102500, 104101, 103901, 103700 | 1,001,000,000 (on), 3,001,000,000 (off) |
| `test-range-ends.yaml` | -40, 0, 125, 25.37 | 100, 500, 1750, 753 | 96000, 100000, 112501, 102530 | 2,001,000,000 (on), 3,001,000,000 (off) |
| `test-below-threshold.yaml` (negative control) | 25, 35, 39.99 | 750, 850, 899 | 102500, 103500, 103990 | none |

## Measured result

`labwired test`, debug CLI. All three scripts exit 0 on their first run with 15, 11 and 9 assertions. Every value above was observed as calculated.

- Oscilloscope: `sensor_mv` (channel `source-u4-mcp9700.voltage_mv`, mV) crosses 900 mV rising at 1,001,000,000 ns and 880 mV falling at 3,001,000,000 ns in the alarm script. The runs keep all 4,001, 4,001 and 3,001 oscilloscope samples (none dropped). In the negative control its maximum is 899 mV and it has no 900 mV crossing.
- Logic analyzer: `user_led_drive` and `user_led_state` edges at the times above; hysteresis keeps the indicator on at 39 °C.
- Console, alarm script: `T=25.0 C`, `T=41.0 C ALARM`, `T=39.0 C ALARM`, `T=37.0 C`. Range script: `T=-40.0 C`, `T=0.0 C`, `T=125.0 C ALARM`, `T=25.3 C` (753.7 mV truncates to 753 mV).
- Mutation check of the negative control (a `/tmp` copy, not committed): at 40.00 °C instead of 39.99 °C, 6 of its 9 assertions fail. The LED drive edge appears at 2,001,000,000 ns and `sensor_mv` crosses 900 mV.
- Wall time, debug CLI, single runs: 1.27 s, 0.97 s and 0.86 s for 4, 4 and 3 s of virtual time.

Browser run: see [browser-flex-scripts.json](../browser-flex-scripts.json).

### Gap found

The first version evaluated U4 every 1 µs, as the other HeyPCB fixtures do. All three scripts passed, but the generic sweep test `every_flex_fixture_runs_unchanged_through_the_generic_runner` failed: it requires `analog_dropped == 0`, and the oscilloscope ring keeps 20,000 samples. A 1 µs analog model over a 4 s run produces 4,000,001 samples; the saved `flex-trace.json` held only the last 20 ms and reported `dropped: 3980001`. The assertions read the full run and were not affected. The fixture now evaluates U4 every 1 ms. A generic fix is still open: a 1 µs analog model cannot run longer than 20 ms without dropping trace samples, although its output is a held step signal that a change-only trace would keep whole.

## Onboarding effort

Measured once on 2026-11-20, approximate, from file and command times. About 9 min from the first listing of the pinned board folder to the descriptor, firmware, scripts and source map (data sheet download and pin-table check included). The debug builds then took about 30 min at a machine load average of about 43, shared with other lanes. Three native scripts passed on their first run; the 1 ms step change above took about 15 min more, most of it the CLI sweep test. No comparable time exists for a chip-specific model of this board, so these numbers support no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the ADC peripheral, sensor accuracy, or chip parity.
