# Smart basket native power testbench

This example exercises LabWired's existing native `external_process` adapter
with a deterministic, bounded, reduced-order **power-only** plant. It represents
an assumed 1S cell, USB load-first power path, charger, 5 V boost output,
3.3 V buck-fed sensor load, and latched rail protection. The chip descriptor is
required manifest scaffolding: **no firmware, QEMU guest or Linux image runs**.

The cell curve, source resistance, converter losses, rail regulation and
protection behavior below are explicit assumptions. They are not calibrated
against a vendor cell/charger/converter, an authoritative application circuit,
or a physical carrier. An application-generated parameter fixture can replace
the public defaults; doing so does not make the assumed cell or protection
model vendor-exact. Do not use the results as Linux/RF/thermal/physical-hardware
acceptance or converter switching stability evidence.

## Run

From the repository root, with the existing native `labwired` binary and
`libngspice.so.0` available:

```bash
python3 -m pytest tools/cosim/test_smart_basket_power.py tools/cosim/test_labwired_ngspice.py -q
python3 examples/smart-basket-power/run_scenarios.py

target/debug/labwired cosim-step examples/smart-basket-power/system.yaml \
  --steps 10 --json --set basket.usb_present=true \
  --set basket.host_load_w=2.0 --set basket.config.initial_soc=0.7 \
  --set basket.config.usb_limit_a=0.6

target/debug/labwired cosim-step examples/smart-basket-power/system.yaml \
  --steps 10 --json --set basket.usb_present=false \
  --set basket.host_load_w=2.0 --set basket.config.initial_soc=0.7 \
  --set basket.config.usb_limit_a=0.6
```

The driver runs seven shared JSONL request streams: USB detach/reattach, radio
load burst, low-battery request/acknowledgment, shutdown timeout, output-rail
short/recovery, independent undervoltage cutoff, and full-cell charge
termination. Each stream is replayed in two fresh model processes and compared
byte-for-byte. It writes requests, actual output JSONL, a 420-row CSV, and a JSON
report containing every assertion. Any failed assertion or adapter/runner error
returns nonzero. Assertion failures produce `report.json` with `passed: false`.
A missing binary/library or malformed adapter response produces an error on
stderr and nonzero exit; no success report is fabricated.

Two independent native CLI processes run supplied-USB and battery-only cases
through manifest routing and the real `CosimRunner`/`ExternalProcessAdapter`
chain. Their raw JSON and command arguments are retained. These constant-input
native checks are separate from the changing-input JSONL replay; the CLI has
no changing-signal scenario playback API in this example.

The native CLI defaults to `target/debug/labwired`; override with
`--native-cli /path/to/labwired`. No Rust build is invoked by the driver. A pytest
native-runner test skips if that binary is absent; the full scenario driver
requires it and fails if absent.

Application-owned parameters may be supplied as a JSON object with public
parameter names, for example:

```json
{"capacity_mah": 3000, "usb_limit_a": 1.0, "charge_limit_a": 0.5}
```

```bash
python3 examples/smart-basket-power/run_scenarios.py \
  --parameters /path/to/application-power-parameters.json \
  --output /path/to/application-evidence
```

Unspecified keys retain documented assumptions. The report records supplied
parameters and intentional low/fault test overrides. Native smoke tests use
explicit SOC 0.7 and USB limit 0.6 A to check routing independently; they do not
claim to exercise every supplied application parameter.

## JSONL and manifest contract

Each request contains exactly `time_ns`, `dt_ns`, and an `inputs` object. The
reply contains `outputs`. `time_ns` is the interval end; the first interval starts
at zero. Each following time must equal the previous time plus `dt_ns`. Times
must be positive integer nanoseconds; `dt_ns` is at most 60 seconds and `time_ns`
is at most `2^63-1`. Duplicate, backward, gapped, nonfinite, malformed, and
out-of-bounds requests fail. Invalid requests do not advance model state.

Parameters are **flat named inputs**, such as `config.capacity_mah`, routed
from `basket.config.capacity_mah`. The external adapter does not forward
arbitrary manifest `config` to the model. Putting parameters only under the
manifest `config` would not configure this model. Parameters become immutable
at the first valid step; repeat identical values or omit them later.

| Parameter input | Default assumption | Accepted range |
| --- | ---: | ---: |
| `config.capacity_mah` | 2000 mAh | 100–20000 |
| `config.initial_soc` | 0.6 | 0–1 |
| `config.usb_limit_a` | 1.5 A | 0–3 |
| `config.charge_limit_a` | 0.5 A | 0–2 |
| `config.boost_efficiency` | 0.90 | 0.5–1 |
| `config.buck_efficiency` | 0.90 | 0.5–1 |
| `config.low_battery_v` | 3.4 V | 3–4 |
| `config.cutoff_v` | 3.1 V | 2.5–3.5, below low-battery threshold |
| `config.shutdown_timeout_s` | 5 s | 0.1–120 |

Boolean inputs are `usb_present`, `shutdown_ack`, `enable`, `fault_short`, and
`reset`; numbers and strings are not accepted as booleans. Defaults are false,
except `enable`, which defaults true. Load inputs default to zero:
`host_load_w` (0–20 W), `radio_load_w` (0–10 W), and `sensor_load_w` (0–5 W).
Host and radio loads are measured at the assumed 5 V output; sensor load is
measured at the 3.3 V output. Nonfinite numbers, boolean loads and unknown keys
fail instead of being silently coerced or ignored. Omitted ordinary inputs
return to their defaults each interval; they are not retained.

Outputs route under `basket.power.*`. `battery_current_a` and
`battery_power_w` are positive during discharge and negative during charging.
`charge_current_a` is nonnegative. `soc`, `battery_v`, `rail_5v`, `rail_3v3`,
`shutdown_request`, `cutoff_latched` and `fault_reason` expose end state.
`battery_terminal_interval_v`, currents and powers describe the interval's
constant-current approximation. `requested_load_w` is output-side host + radio
+ sensor demand; `rail_input_load_w` includes buck loss. `delivered_load_w` and
`delivered_rail_input_w` are interval-average deliveries, which may be nonzero
in an interval whose final battery voltage trips the cutoff. `usb_current_a`,
`usb_input_power_w`, and `power_balance_error_w` complete the power ledger.

## Electrical and protection assumptions

The piecewise-linear cell open-circuit curve uses `(SOC, volts)` points
`(0,3.0), (0.05,3.3), (0.15,3.55), (0.5,3.75), (0.9,4.05), (1,4.2)`.
Series resistance is fixed at 0.12 Ω. Battery current comes from the stable
quadratic solution to `P = I * (OCV - I*R)` during discharge. A requested draw
above 4 A or beyond the source's constant-power capability trips
`source_overload`; these are assumed protective limits, not a component rating.
SOC integrates signed current by coulomb counting with `capacity_mah * 3.6`
ampere-seconds; SOC cannot leave 0–1.

USB voltage is fixed at 5 V. The USB-to-output path and charge stage each have
assumed efficiency 0.95. USB serves output demand first; the remaining budget
may charge the cell, capped by `charge_limit_a`. If demand exceeds that budget,
the battery supplies the deficit through the boost efficiency and no charging
occurs. The charge stage solves a bounded CC/CV approximation, including
end-of-interval SOC and resistance, so terminal voltage cannot exceed 4.2 V;
a full cell receives zero charge. Boost and buck efficiency are fixed input
parameters; there is no operating-point efficiency curve or ripple.

Low loaded cell voltage latches a software shutdown request while the battery
path is required. The timeout is a **software-policy assumption with no firmware
implementation or automatic hardware timer demonstrated**; the proposed
supervisor CT delays recovery and does not implement this shutdown timeout. A request followed by `shutdown_ack`, or reaching the timeout,
latches the rail cutoff. The independent cell voltage cutoff does not need a
software acknowledgment and also blocks USB-powered output rails when the cell
is depleted, matching the proposed battery-monitored latch topology. `fault_short` represents an **output-rail short** and
immediately latches both rails off; charging is inhibited while the short
input is asserted. It does not calculate short-circuit current or damage.

A cleared short, reappearing USB, or toggling `enable` never clears a cutoff.
Only explicit `reset: true`, with no asserted short and a recovered source,
clears it. Recovery always requires open-circuit cell voltage at least
`low_battery_v + 0.2 V`, plus USB budget covering the requested load or loaded
battery voltage above the same recovery threshold and an acceptable draw.
USB alone cannot reset a depleted cell. The public inputs accept the proposed
TPS3840DL33 3.3 V warning / DL30 3.0 V cutoff fixture thresholds; with a 3.3 V
warning the assumed recovery boundary is 3.5 V. Supervisor tolerance and the
LTC2954 400–650 ms startup KILL-ignore interval are not modeled. USB may
charge a cutoff cell after the rail short is cleared. `enable: false` removes
output load while leaving an available USB charger active.

Protection and loads are sampled at interval boundaries. Source equations use
beginning SOC and SOC updates once per interval. This is a reduced-order,
constant-current interval approximation, not transient protection timing.
The supplied manifest uses 100 ms steps; the 60 s protocol ceiling is a resource
bound and should not be treated as useful shutdown timing resolution.

## Independent ngspice check

`divider-rc.cir` contains assumed 100 kΩ/100 kΩ divider resistors and a 100 nF
ADC filter. `models/divider.py` uses the existing `labwired_ngspice.py` wrapper
with real libngspice. A 4 V step is checked against
`2 * (1 - exp(-t / 5 ms))` at five points, with 10 mV absolute tolerance.
This checks the measurement divider/RC only; it does not validate the charger,
boost, buck, cell model, load waveforms, or application schematic.

See [VERIFICATION.md](VERIFICATION.md) and the [recorded report](artifacts/report.json)
for the current local evidence. The stored artifacts use default assumptions.
