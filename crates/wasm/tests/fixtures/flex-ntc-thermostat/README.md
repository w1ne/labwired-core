# Held-out NTC fan thermostat (setup/loop) in Flex

This portable workload has the usual firmware shape: `setup` runs once and `loop` runs once per control period. It uses the existing [`ntc-thermistor` analog source](../../../../../configs/devices/ntc_thermistor.yaml) unchanged: a 10 kΩ, B=3950 thermistor above a 10 kΩ pull-down divider. The firmware turns a fan on at 2000 mV and off at 1800 mV. The fan state is a firmware static, so the result at a temperature inside the band depends on the history.

`labwired test` runs the firmware through a `sequence`: firmware calls and test-bench temperature steps on one firmware instance and one runtime.

| Script | Temperature steps (°C) | Sensor (mV) | `loop` result after each step | Fan edges |
| --- | --- | --- | --- | --- |
| `test-heat-cycle.yaml` | 25, 50, 33, 25 | 1650, 2428, 1932, 1650 | 0, 1, 1, 0 | drive 1 at 3000 ns, state 1 at 4000 ns, drive 0 at 7000 ns, state 0 at 8000 ns |
| `test-mild.yaml` (negative control) | 25, 33, 25 | 1650, 1932, 1650 | 0, 0, 0 | none |

At 33 °C the sensor reads 1932 mV in both scripts. The fan is on in the heat cycle and off in the mild case. A single stateless call cannot produce this difference.

`crates/cli/tests/flex_test_runner.rs` checks each step result and time, the four digital edges, and the nine oscilloscope samples of the heat cycle. It also checks that the heat-cycle expectations fail on the mild temperature steps, that a contract mismatch in a late step is a configuration error before step 0, that a temperature outside the model range (-55..300 °C) stops the run with a model error, and that a limit inside a repeated call leaves that step without a result.

## Scope

This is functional behavior against a reusable analog source. It is not original target firmware, target ADC fidelity, component tolerance, a thermal model, a physical measurement, or a full-board simulation. One `loop` call is one 1 µs control period; the period is a test value. There is no browser run and no `native.json` for this fixture: the evidence is the command-line runner only.

## Reproduce

From the repository root:

```sh
sh crates/wasm/tests/fixtures/flex-ntc-thermostat/build.sh
cargo run -p labwired-cli --bin labwired -- test --no-key \
  --script crates/wasm/tests/fixtures/flex-ntc-thermostat/test-heat-cycle.yaml \
  --output-dir /tmp/thermostat --vcd /tmp/thermostat/digital.vcd \
  --analog-trace /tmp/thermostat/analog.csv
cargo test -p labwired-cli --test flex_test_runner
```
