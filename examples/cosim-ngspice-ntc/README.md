# ngspice NTC divider (`adapter: ngspice`)

A negative-temperature-coefficient thermistor in a divider, solved by ngspice
in lockstep with the simulated clock. The NTC is a behavioral (`B`) source,
`R = R25 * exp(B * (1/(T + 273.15) - 1/(T25 + 273.15)))`, which the in-core
analog engine cannot parse. This circuit therefore needs ngspice; the in-core
engine offers the same curve as a `config.thermistors` entry.

| File | Use |
|---|---|
| `ntc.cir` | The netlist. `Vtemp` is the ambient temperature (1 V = 1 °C). |
| `flex-ntc.yaml` | CPU-free Flex: `ambient_c` write port, `sensor_v` (V) read port. |

The part is the NCP18XH103F03RB row of Murata catalog R44E: R25 = 10 kOhm,
B25/50 = 3380 K. R6 is 10 kOhm from 3V3 to SNS_AN1, U4 is the NTC from SNS_AN1
to GND, and C7 is 100 nF across the node. The same network and values are used
by the pinned HeyPCB ESP32-S3 NTC Flex fixture.

Expected settled node voltage: 2.131089 V at 10 °C, 1.650000 V at 25 °C and
0.769167 V at 60 °C.

The deck's curve is stated for 10-60 °C, so the manifest declares that domain
with `config.input_ranges.temperature` (min 10, max 60, unit `°C`). A
temperature outside it stops the run:

    ngspice model 'ntc' input 'temperature' is 61, outside the declared 10..60 °C range

`cosim_ngspice::a_thermistor_deck_matches_the_in_core_curve_within_its_range`
is the differential evidence: the same beta curve as the in-core `thermistors`
entry agrees at 10, 25 and 60 °C, and both engines refuse 61 °C.
`cosim_ngspice::flex_ntc_divider_runs_on_native_ngspice` runs this manifest
through `FlexRuntime`, checks the named port and the oscilloscope channel, and
checks the range stop.

Native requirements: `python3` and libngspice (`apt install libngspice0`).
The browser runs a Flex ngspice model through
`WasmFlexRuntime::new_from_yaml_with_ngspice_callback` with the netlist text
supplied by the caller; this example is a native proof until the app supplies
that callback for a Flex project.

Tests (skip without libngspice; `LABWIRED_REQUIRE_NGSPICE=1` makes that a
failure):

    cargo test -p labwired-core --features flex-script --test cosim cosim_ngspice::

`model:` resolution: `FlexRuntime::from_yaml` resolves it from the process
working directory; `FlexRuntime::from_yaml_with_base` and
`labwired test --script` resolve it from the Flex manifest's directory.
