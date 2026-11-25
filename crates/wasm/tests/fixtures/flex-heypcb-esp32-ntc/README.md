# HeyPCB ESP32-S3 NTC on a Flex analog network

Portable Rust-to-Wasm firmware reads the NTC divider of the pinned **HeyPCB ESP32-S3 NTC** board, revision A. The in-core analog engine solves the board's own sensor network: R6 (10 kΩ) from 3V3 to `SNS_AN1`, U4 (Murata NCP18XH103F03RB) and C7 (100 nF) from `SNS_AN1` to GND. U4 is a netlist resistor bound to the analog adapter's existing beta-curve `thermistors` key. Ten times a second the firmware reads `SNS_AN1` in millivolts on IO1, computes the U4 resistance from the divider, converts it to a temperature with the beta equation, prints it to a Flex console that stands for the board's native USB serial port, and drives the IO48 user LED as an over-temperature indicator. No ESP32-S3 CPU or ADC peripheral is emulated.

This board is a held-out workload. The catalog `ntc-thermistor` descriptor is a different part in a different circuit (B = 3950 K above a 10 kΩ pull-down), so it was not used. The board needed no Rust model code, no new manifest section, tool, importer, MCP command or board-specific code. It found two generic defects, fixed in shared code:

- The Flex manifest check refused `config.thermistors` (`unsupported config key 'thermistors'`), so no Flex manifest could place an NTC in a circuit. It now accepts the key with the same route rules as `photoresistors`.
- The analog adapter reported its thermistor input unit as `degC`, while every catalog `analog_source` temperature channel and every Flex temperature port spells it `°C`. A `°C` port was refused with `no implicit unit conversion`. The adapter now reports `°C`.

## Source

- Board: [HeyPCB/esp32-sensors-1 at `57a2122a`](https://github.com/HeyPCB/esp32-sensors-1/tree/57a2122a19ac1aa9c092e43fb5b123c461508752/boards/ntc), asset `boards/ntc/kicad/esp32_ntc`, license CERN-OHL-S-2.0. [source-map.json](source-map.json) records the Git blob IDs; each one was checked with `git hash-object` on the cloned file. A regression checks the revision and license against the sibling LDR fixture's source map.
- Network: the pad nets come from the pinned PCB blob. R6 pad 1 is on `3V3`, pad 2 on `SNS_AN1`; U4 pad 1 on `SNS_AN1`, pad 2 on GND; C7 pad 1 on `SNS_AN1`, pad 2 on GND. U3 pad 39 (IO1) is on `SNS_AN1`, U3 pad 25 on IO48. The README states "under a 10k resistor from 3.3 V, so IO1 reads 1.65 V at 25 C and falls as it warms ... 100 nF across it".
- Part: Murata catalog R44E, *NTC Thermistors* (Dec.17,2012), as LCSC serves it for C13564, sha256 `64c596ad…e59ef5` (full value in the source map). NCP18XH103F03RB row (PDF page 12): R25 = 10 kΩ ±1 %, B25/50 = 3380 K ±1 %, B25/85 = 3434 K (reference), operating range -40..125 °C, dissipation constant 1 mW/°C. Center-value table for NCPppXH103 (PDF page 17).
- The source README states that the board was not fabricated, assembled, or powered. The board folder contains no firmware.

## Model

```yaml
netlist_text: |
  V3V3 3v3 0 dc 3.3
  R6 3v3 sns_an1 10k
  RU4 sns_an1 0 10k
  C7 sns_an1 0 100n
thermistors:
  temperature: { resistor: RU4, r_ref_ohms: 10000, beta_kelvin: 3380, reference_c: 25, min_c: 10, max_c: 60 }
```

The declared range is a decision from the catalog table, not the part's operating range. Against Murata's center values the beta equation with B25/50 is within 1.7 % (0.40 °C) from 10 °C to 60 °C, but +3.7 % at 0 °C, +3.1 % at 85 °C, +20.5 % (+3.0 °C) at -40 °C and +9.2 % (+4.2 °C) at 125 °C. A temperature outside 10..60 °C stops the run with an explicit error. A regression recomputes these deviations from the table in the source map.

The analog engine steps every 100 µs with 10 backward-Euler substeps. The firmware ([firmware.rs](firmware.rs)) samples 10 ms into each 100 ms period, which is at least 15 time constants of C7 against R6 ‖ U4 in the declared range. It takes whole millivolts (truncation), as `analogReadMilliVolts()` returns them, and computes `T = 1 / (1/298.15 + ln(R/10 kΩ) / 3380) - 273.15` with its own `ln`. `loop` returns (10,000 + T in tenths) × 10 + LED. The thresholds are on at 40.0 °C and off below 38.0 °C. Because the firmware uses the same beta equation as the model, a pass does not test the beta equation against the real part.

The same network and part values have an ngspice deck in [`examples/cosim-ngspice-ntc`](../../../../examples/cosim-ngspice-ntc/README.md). `cosim_ngspice::a_thermistor_deck_matches_the_in_core_curve_within_its_range` compares that deck's `B` source with this `thermistors` entry at 10, 25 and 60 °C. This fixture now carries its own reference-solver variant, [`system-ngspice.yaml`](system-ngspice.yaml) with [`ntc.cir`](ntc.cir), and three `ngspice-*.yaml` scripts; see [Reference solver](#reference-solver-ngspice) for the measured parity and the one transient difference.

## Expected values, calculated before the first run

A test-bench `set` at a period start latches at the next 100 µs boundary. An LED write reaches `user_led_state` 1,000 ns later.

| Script | Ambient (°C) | SNS_AN1 settled (V) | Firmware mV | `loop` results | LED drive edges (ns) |
| --- | --- | --- | --- | --- | --- |
| `test-overtemp-alarm.yaml` | 26, 42, 38, 36 | 1.618740, 1.160648, 1.266386, 1.321658 | 1618, 1160, 1266, 1321 | 102600, 104201, 103801, 103600 | 110,000,000 (on), 310,000,000 (off) |
| `test-range-ends.yaml` | 10, 60, 25, 33.3 | 2.131089, 0.769167, 1.650000, 1.398660 | 2131, 769, 1650 (observed 1649), 1398 | 101000, 106001, 102500, 103330 | 110,000,000 (on), 210,000,000 (off) |
| `test-below-threshold.yaml` (negative control) | 26, 36, 39.9 | 1.618740, 1.321658, 1.215343 | 1618, 1321, 1215 | 102600, 103600, 103990 | none |

C7 transient in the alarm script: the 42 °C step latches at 100.1 ms. The time constant is (10 kΩ ‖ 5425 Ω) × 100 nF = 352 µs, so the node passes the 50 % level (1.38969 V) 244 µs later; the first sample at or below it is at 100.4 ms.

## Measured result

`labwired test`, debug CLI, 2026-11-21. The alarm and negative-control scripts pass on their first run with 15 and 7 assertions. The range script failed one assertion on its first run:

- Prediction miss: at 25 °C after 60 °C the firmware printed `mV=1649`, not `mV=1650`. The settled value, 1.650000 V, is exactly on a millivolt boundary, and the calculation did not flag that margin. After the rise from 0.769 V the backward-Euler node is still about 2.7 nV below 1.65 V 10 ms later (0.881 V × (1/1.02)^990); the firmware truncates the f64 value. 1649 mV still converts to 25.0 °C, and the `loop` result 102500 was as calculated. The script now asserts the observed text and explains it. Every other reading had at least 0.02 mV of margin to its millivolt boundary.
- Oscilloscope: `sensor_v` is 1.65 V at 0 ns (the 25 °C port default), passes 1.38969 V falling at 100,400,000 ns, and stays between 1.16064 V and 1.65001 V. The runs keep all 4,001, 4,001 and 3,001 samples (none dropped).
- Logic analyzer: `user_led_drive` and `user_led_state` edges at the times above; hysteresis keeps the indicator on at 38 °C.
- Console, alarm script: `T=26.0 C mV=1618`, `T=42.0 C mV=1160 ALARM`, `T=38.0 C mV=1266 ALARM`, `T=36.0 C mV=1321`.
- Wall time, debug CLI, single runs: 0.40 s, 0.33 s and 0.24 s for 0.4, 0.4 and 0.3 s of virtual time.

Negative controls (`/tmp` copies, not committed):

- 40.0 °C instead of 39.9 °C in the below-threshold script: 6 of 7 assertions fail. `loop` returns 104001, the node reaches 1.21270 V and crosses 1.2127 V, and both LED signals get one edge.
- 61 °C instead of 60 °C in the range script: exit 3, `stop_reason: model_error`, message `analog input `temperature`: thermistor temperature must be within 10..60 C`, at 100,000,000 ns; 8 of 10 assertions fail.
- B = 3950 K, the catalog `ntc-thermistor` value, instead of 3380 K: 9 of 15 alarm-script assertions fail. The firmware reads 1084 mV at a 42 °C ambient and prints `T=45.0 C`.
- Without the manifest-check fix the manifest stops with `unsupported config key 'thermistors'`; without the unit fix it stops with `requires unit degC; signal 'environment.temperature' declares Some("°C")`.

Browser run: Chromium 156.0.8078.79 runs all three scripts at `1a59956f12` with the same result text, logic, oscilloscope, analog series and console records as the CLI. See [browser-flex-scripts.json](../browser-flex-scripts.json).

## Reference solver (ngspice)

`system-ngspice.yaml` is the same project with the R6/U4/C7 network solved by
ngspice: `ntc.cir` writes U4 as the behavioural current source
`Bntc = V(sns_an1) / (10k · exp(3380 · (1/(273.15 + V(temp)) − 1/298.15)))`, and
the model declares the same 10..60 °C `input_ranges` domain. Ports, firmware,
the IO48 behavior model, the console and the model sequence are unchanged. The
three `ngspice-*.yaml` scripts repeat the in-core sequences with the same
assertions; they are not named `test-*.yaml` because the generic fixture matrix
and the browser receipt have no solver host. The native host needs python3 and
libngspice (`apt install libngspice0`).

Measured on 2026-11-25 with ngspice-42 through `labwired test --script`:

| Observation | In-core | ngspice |
| --- | --- | --- |
| Scripts passing with the same assertions | 3 of 3 | 3 of 3 |
| `loop` results, alarm script | 102600, 104201, 103801, 103600 | identical |
| Console text, range script | `mV=2131 / 769 / 1649 / 1398` | identical |
| Scope samples, alarm / range / negative | 4001 / 4001 / 3001 | identical |
| Settled scope deviation | — | ≤ 2·10⁻⁷ V (f32) |
| Fastest transient sample at 100.3 ms (analytic 1.346593 V) | 1.357021 V | 1.346538 V |

- The 1.05·10⁻² V gap in the fastest transient is the in-core engine's
  first-order backward-Euler lag; ngspice stays within 0.1 mV of the analytic
  curve. Both cross the alarm level 1.38969 V at the 100.4 ms sample, and every
  sample the firmware reads (10 ms into each period) is identical, including
  the 1649 mV truncation case.- 61 °C stops both engines with `status: error`, `stop_reason: model_error`:
  `analog input `temperature`: thermistor temperature must be within 10..60 C`
  in-core and `ngspice model 'source-sns-an1' input 'temperature' is 61,
  outside the declared 10..60 °C range` from the shared latch.
- The run report pins the deck: `source.path: "./ntc.cir"`, `bytes` and
  `sha256`, next to `manifest_sha256` of `system-ngspice.yaml`.

Reproduce:

```bash
labwired test --script crates/wasm/tests/fixtures/flex-heypcb-esp32-ntc/ngspice-overtemp-alarm.yaml --output-dir /tmp/ntc-alarm
LABWIRED_REQUIRE_NGSPICE=1 cargo test -p labwired-cli --test cosim_pin_routing labwired_test_runs_the_heypcb_ntc_deck_on_ngspice
```

The regression test runs all three scripts on both engines and requires equal
`entry_result`, console and instrument counts, the netlist pin, and both range
stops. The browser keeps the in-core manifest: the committed browser receipt
has no ngspice host, and a page must supply the ngspice WASM callback before
this deck can run there. This is not a chip-parity claim and does not prove the
part's tolerance or a physical board.

## Onboarding effort

Measured once on 2026-11-21, from command times in one lane session. About 27 min wall time from the clone of the pinned board folder (04:34 UTC) to three passing scripts (05:01 UTC), including the catalog search, both shared-code fixes and two debug CLI builds (8 min 11 s for the first). No comparable time exists for a chip-specific model of this board, so these numbers support no speed claim.

A pass shows that this firmware satisfied the assertions against the declared model. It is not evidence for ESP32-S3 firmware, the ADC peripheral, the NTC's accuracy, or chip parity.
