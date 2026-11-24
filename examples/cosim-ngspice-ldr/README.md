# ngspice LDR divider (`adapter: ngspice`)

A light-dependent resistor in a divider, supplied by a firmware pin and solved
by ngspice in lockstep with the simulated clock. The LDR is a behavioral (`B`)
source, `R = 10 kOhm * (E / 10 lx)^-0.7`, which the in-core analog engine
cannot parse. This circuit therefore needs ngspice.

| File | Use |
|---|---|
| `ldr.cir` | The netlist. `Vsup` is the divider supply, `Vlux` is the light level (1 V = 1 lx). |
| `system.yaml` | NUCLEO-F401RE: blink firmware drives PA5 into `Vsup`, `ui.ldr.lux` sets the light, the node goes to PA0. |
| `flex-ldr.yaml` | CPU-free Flex: `enable` and `light` write ports, `ldr_volts` (V) and `dark` (threshold 1.65 V) read ports. |

Expected settled node voltage: 0.548914 V at 100 lx, 2.751086 V at 1 lx.

The deck's curve is stated for 1-100 lx, so both manifests declare that domain
with `config.input_ranges.lux` (min 1, max 100, unit `lx`). A light level
outside it stops the run:

    ngspice model 'ldr' input 'lux' is 150, outside the declared 1..100 lx range

The deck keeps `max(V(lux), 0.01)` so the solver stays finite, but the run no
longer reports a plausible voltage for an input the model was not fitted to.
`cosim_ngspice::a_photoresistor_deck_matches_the_in_core_curve_within_its_range`
is the differential evidence for this: the same GL5528 curve as the in-core
`photoresistors` entry agrees at 1, 10 and 100 lx, and both engines refuse
150 lx.

Native requirements: `python3` and libngspice (`apt install libngspice0`).
The browser runs `system.yaml` through
`WasmSimulator::new_from_config_with_ngspice_callback` with the netlist
supplied under the name `./ldr.cir`. `flex-ldr.yaml` also runs in the browser
through `WasmFlexRuntime::new_from_yaml_with_ngspice_callback`; the native and
browser hosts share the `(0, 0)` priming and boundary contract. The real-browser
proof is `scripts/test_browser_ngspice.mjs`. This is a WASM API/host integration,
not yet a Playground Flex user-interface workflow.

Tests (skip without libngspice; `LABWIRED_REQUIRE_NGSPICE=1` makes that a failure):

    cargo test -p labwired-core --features flex-script --test cosim cosim_ngspice::
    cargo test -p labwired-cli --test cosim_pin_routing

Measured on 2026-11-23 with libngspice 42: 14 of 14 `cosim_ngspice::` tests and
9 of 9 `cosim_pin_routing` tests pass. The browser check gives 11 of 11 cases
(Chrome 156.0.8078.79, ngspice-47 WASM); the Flex LDR divider reads 0.548914 V
at 100 lx and 2.751073 V at 1 lx.

`model:` resolution: `labwired test --script` resolves a relative `model:` from
the Flex manifest's directory. `FlexRuntime::from_yaml` (and the CPU-free
`flex-ldr.yaml` test that uses it) resolves it from the process working
directory, so that test substitutes an absolute path. The browser exchange is
the app's `prepareBrowserFlexNgspiceModels` / `createBrowserNgspiceExchange`,
merged in `w1ne/labwired#3897` (`590744b4d2`).
