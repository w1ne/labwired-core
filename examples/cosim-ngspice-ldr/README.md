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

Native requirements: `python3` and libngspice (`apt install libngspice0`).
The browser runs `system.yaml` through
`WasmSimulator::new_from_config_with_ngspice_callback` with the netlist
supplied under the name `./ldr.cir`.

Tests (skip without libngspice; `LABWIRED_REQUIRE_NGSPICE=1` makes that a failure):

    cargo test -p labwired-core --test cosim cosim_ngspice::

Known limit: Flex resolves `model:` from the process working directory, not
from the manifest directory. The test substitutes an absolute path.
