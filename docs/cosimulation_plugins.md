# Co-Simulation Plugins

LabWired owns deterministic firmware execution, board topology, traces, and
fault orchestration. Physical plant models can be declared as co-simulation
models and stepped through an adapter.

The first supported manifest shape is:

```yaml
cosim_models:
  - id: "plant_model"
    adapter: "external_process" # external_process | fmi | mock
    model: "./models/mock-plant.py"
    step_ns: 10000
    inputs:
      controller_enable: "control.enable"
      load_torque_nm: "plant.load.torque_nm"
    outputs:
      shaft_speed_rpm: "observables.shaft_speed_rpm"
      plant_ready: "observables.plant_ready"
    config:
      protocol: "jsonl"
```

`external_process` and `fmi` adapters require `model`. `step_ns` must be
greater than zero. The `mock` adapter is intended for deterministic tests and
dry-runs; its static outputs are declared under `config.outputs` so the top-level
`outputs` map can stay dedicated to LabWired signal routing.

The core runtime now exposes a small co-sim registry and runner:

- `build_cosim_adapter(config)` constructs `mock` and `external_process`
  adapters from a `CosimModelConfig`.
- `adapter: fmi` intentionally returns a clear unsupported error until the FMI
  import path is selected.
- `CosimRunner::step_until(time_ns, inputs)` steps models only at their
  configured `step_ns` boundaries.
- `CosimRunner::step_until_with_signals(time_ns, signals)` applies the manifest
  `inputs` and `outputs` maps against a signal store. For example,
  `controller_enable: control.enable` feeds the model-local
  `controller_enable` input from the `control.enable` store path, and
  `shaft_speed_rpm: observables.shaft_speed_rpm` writes the model output back
  to `observables.shaft_speed_rpm`.
- `CosimRunner::from_configs_with_base(configs, base_dir)` resolves relative
  external model paths against the manifest directory, so example manifests can
  keep local `./models/...` references.
- `cosim::routing` maps store paths onto real machine state — see
  [Board signal paths](#board-signal-paths) — and `cosim::CosimSession` is the
  runner, the routing, and the cycle ↔ nanosecond time base bound to one
  machine, which is what `labwired test` steps.

This contract is intentionally domain-neutral. A model can represent a motor,
thermal plant, hydraulic system, sensor array, battery pack, power stage, or
any other external process as long as it consumes named inputs and returns named
outputs.

The `examples/cosim-plant-demo` manifest exercises this generic contract with a
reduced-order discrete plant: per-channel enabled/disabled states, a
scenario-driven `disabled_channels` list, and voltage/current/active-channel
observables. Higher-fidelity behavior (loss, thermal, semiconductor stress,
grid-compliance validation) belongs in a later adapter-backed model.

Drive any manifest-declared model from the command line with
`labwired cosim-step <system.yaml> --set <path>=<value>`, which builds the
runner from the manifest and prints the routed outputs after stepping.

## Mixed-signal: ngspice as a model

`tools/cosim/labwired_ngspice.py` turns any SPICE netlist into an
`external_process` model. It hosts libngspice in-process, and on every step
alters the mapped voltage sources to the requested values, places a breakpoint
at the step's end time and resumes the transient analysis to it; the probed
node voltages come back as outputs. There are no threads and no wall clock, so
the same input sequence always produces the same voltages.

- Inputs: `true`/`false` map to `vdd` / 0 V (a GPIO pin), numbers are volts.
- Outputs: last value of the probed vector (`v(node)`, `i(vsrc)`) at step end.
- The operating point is solved from the netlist's own defaults first; inputs
  are applied once time is running, like a real pin edge.
- Open-source and vendor device models work through the netlist's `.include`
  and `.lib` lines; check each library's licence before bundling it.
- One circuit per wrapper process (libngspice is process-global). A second
  circuit is a second `cosim_models` entry.

Worked example: [`examples/cosim-spice-rc`](../examples/cosim-spice-rc/README.md)
(GPIO into a 10 kΩ / 100 nF low-pass, τ = 1 ms). Requires `libngspice0`
(Debian/Ubuntu: `apt install libngspice0`). Tests:
`python3 -m pytest tools/cosim/test_labwired_ngspice.py`.

## Board signal paths

A model that only speaks abstract observables can be driven from the command
line but never by firmware. The `board.` / `adc.` path grammar is how a
`cosim_models:` entry names something real on the chip, so `labwired test`
steps the model against the pins the firmware is actually driving:

| Path | Direction | Type | What it is |
|------|-----------|------|------------|
| `board.gpio.<pad>` | machine → model | bool | The level the firmware is driving on an output pad. |
| `board.gpio_output.<pad>` | machine → model | bool | Whether the firmware has configured `<pad>` as a general-purpose output. |
| `board.gpio_in.<pad>` | model → machine | bool or volts | An externally held level on an input pad (also readable). A number is volts, turned into a level with the chip's input thresholds. |
| `board.analog.<pad>_volts` | model → machine | number | The analog level on the ADC channel belonging to `<pad>`. |
| `adc.<peripheral>.<channel>_volts` | model → machine | number | The analog level on an explicitly named ADC channel. |
| `ui.<partId>.<field>` | outside → model | bool or number | Set from outside the engine (a canvas part, a test stimulus); 0 / false until set. |

`<pad>` is a pad label in whatever form the chip speaks — `pa5` / `PA5` on
STM32, `pd2` on the ATmega328P, `p0.13` on Nordic, `gpio5` or a bare `5` on
ESP32. Labels resolve through the same pin resolution every other
pad-addressed feature uses: a chip's declared `pins:` map first, then the
standard STM32/Nordic parse, then the ATmega `port<letter>` windows, then the
ESP32 forms. Case does not matter. Board aliases such as Arduino `D2` are not
pad labels; the playground resolves them before a manifest is written.

`board.gpio_output.<pad>` reads the GPIO model's direction register through
`Peripheral::read_gpio_is_output`, the same register truth the logic
analyzer's pin routing reads: ATmega `DDRx`, STM32 `MODER` (F1 `CRL`/`CRH`),
nRF `DIR`, Kinetis `PDDR`, SAM `DIR`/`PINCFG`, EFR32 mode nibbles, and on the
ESP32 family `GPIO_ENABLE` plus the output-matrix selector. It is `true` only for a plain GPIO output. Input, analog and
alternate-function pads read `false`, because the latch `board.gpio.<pad>`
reads is not what drives them. The two paths together describe a pin
electrically. A circuit that treats `board.gpio.<pad>` as a source needs to
know whether that source is connected, or an input pad would clamp the net to
its idle output latch:

```yaml
inputs:
  drv_pa0: board.gpio.pa0          # level of the driver
  ctl_pa0: board.gpio_output.pa0   # the driver's switch control
config:
  netlist_text: |
    Vdrv_pa0 drv_pa0 0 dc 0
    Sdrv_pa0 drv_pa0 v_rc ctl_pa0 ron=25 roff=1e12
  sources: { drv_pa0: Vdrv_pa0 }
```

A pad whose GPIO model cannot report direction fails when the session is built
with "the GPIO model that owns pad … does not report pin direction". It never
reads as `false`, which would disconnect the firmware from the circuit for the
whole run. Every GPIO family `board.gpio.<pad>` resolves on reports it. A pad
`board.gpio.<pad>` cannot resolve (RP2040 pads, for example) fails the same way
for both paths, as "does not resolve on this chip".

On the ATmega328P the AVR core owns `PINx`/`DDRx`/`PORTx` and mirrors them to
the descriptor's `portb`, `portc` and `portd` windows, which is where both
paths read them; an 8-bit port has no `pd8`.

Direction is enforced. `board.gpio.<pad>` is what the firmware drives, so a
model *output* routed to it is a config error rather than a write that silently
does nothing; drive a pin with `board.gpio_in.<pad>` instead, which goes
through the same seam a `board_io` button uses. Likewise an analog path is a
sink only and cannot be read back into a model input.

`board.analog.<pad>_volts` resolves the pad through the chip descriptor's
`analog_pins:` map, transcribed from the datasheet pinout:

```yaml
analog_pins:
  PA0: { peripheral: "adc1", channel: 0 }
```

It is data, not a built-in table, because the assignment differs between
families: PA0 is ADC1_IN0 on an F401, ADC1_IN5 on an L476 and ADC1_IN1 on a
G474. The F1/F4/F7 descriptors in-tree (`stm32f103`, `stm32f401`,
`stm32f401cdu6`, `stm32f405`, `stm32f407`, `stm32f411ceu6`, `stm32f767`) carry
it; the 48-pin packages list PA0..PA7 and PB0/PB1 only, since they bond out no
PC0..PC5. A pad with no entry — every pad on a chip that declares none — fails
at startup with "the chip descriptor names no ADC input for pad …", and
`adc.<peripheral>.<channel>_volts` is the form to use there. `analog_pins:` is
kept apart from `pins:` because `pins:` is the authoritative GPIO map: declaring
it makes every unlisted pad unresolvable.

Both forms write through `SystemBus::seed_adc_channel`, the single choke point
every analog stimulus (thermistor, potentiometer, battery divider) already goes
through; the ADC model owns volts → counts, at 3.3 V full scale and 12 bits
(1.65 V is 2047).

Both forms are also checked against the converter when the session is built.
The named peripheral must exist, must be an ADC, and must have the channel. A
model reports its channel count from its own register layout: 0..=18 for the
STM32 F1/F4 and L4/H5/F7/G0 blocks, 0..=19 for the H7. A failing route is a
startup error that names the path, the peripheral and the valid range, e.g.
`co-sim path 'adc.adc1.99_volts': ADC 'adc1' has channels 0..=18; there is no
channel 99`. Without this check the ADC model would drop a channel it does not
have, and the route would write nothing for the whole run.

Paths outside this grammar — `control.enable`, `plant.output.voltage` — stay
plain signal-store keys routed between models, exactly as before.

### Volts on an input pad

A number routed to `board.gpio_in.<pad>` is a voltage, and the pad is a
Schmitt-trigger input: it reads high once the voltage is at or above VIH, low
once it is at or below VIL, and between the two it keeps the level it read last.
It starts low. A boolean keeps meaning the level itself, exactly as before.
This is what lets a circuit node charging through a megohm reach
`digitalRead` at the right time, instead of flipping at a guessed midpoint.

The thresholds are chip data, transcribed from the datasheet into the chip
descriptor as ratios of the I/O supply:

```yaml
io_voltage_v: 5.0          # the rail the GPIO pads run from
gpio_input_thresholds:     # ratios of io_voltage_v
  vil: 0.3                 # highest input guaranteed to read low
  vih: 0.6                 # lowest input guaranteed to read high
```

In-tree they are declared for `atmega328p` (5.0 V, 0.3 / 0.6, Microchip
DS40002061B Table 30-1), `stm32f103` (3.3 V, 0.35 / 0.65, DS5319 Table 36), and
`stm32f401`, `stm32f401cdu6`, `stm32f405`, `stm32f407`, `stm32f411ceu6` (3.3 V,
0.3 / 0.7, DS10086 Table 54, DS8626 Table 48, DS10314 Table 53). Each
descriptor cites its table. On any other chip, volts routed to a pin fail when
the session is built, naming the missing key:
``co-sim path 'board.gpio_in.pd2' is driven with volts, but the chip descriptor
declares no `gpio_input_thresholds` …``. An `adapter: analog` model's outputs are
always volts, so that check happens before the first step; a model whose value
type is only known at run time gets the same error when the first number
arrives.

### Signals set from outside

`ui.<partId>.<field>` paths belong to neither the machine nor a model. A model
reads one like any other input, every one of them exists from the first step
at 0 / false, and something outside the engine sets it:

```yaml
inputs:
  ctl_touch: ui.touch.pressed      # a canvas touch pad, pressed or not
config:
  netlist_text: |
    Stouch pad finger ctl_touch ron=1k roff=1e12
```

- **Rust:** `CosimSession::set_signal(path, CosimSignalValue)`, or
  `CosimSession::set_signal_number(path, f64)` for a caller that only has a
  number.
- **Browser:** `WasmSimulator::set_cosim_signal(path, value)`; it throws
  rather than returning quietly.
- **Test scripts:** a `cosim_signal` stimulus (below).

A number means what the input it feeds expects. For a logic-level input (an
analog switch control) 0 is false and anything else true, so a press is `1`
whatever the circuit's supply. A voltage or current source takes the number as
given. A value lands in the store at once and every model reading the path
sees it from its next step. It is refused when no model input reads the path
(a typo would otherwise do nothing and report success), when the path is a
board path the machine rewrites every boundary, and for NaN or an infinity. A
model output routed to a `ui.` path is a startup error.

A test script sets one with the stimulus trigger forms every other stimulus
uses:

```yaml
schema_version: "1.2"
stimuli:
  - cosim_signal: { path: ui.touch.pressed, value: 1 }
    trigger: !after_cycles { cycles: 8000000 }
  - cosim_signal: { path: ui.bench.level, value: 2.5 }   # at_start
```

`labwired test` applies it through the run's `CosimSession`, and reports it in
`result.json`'s `stimuli` block with `"cosim_signal": true` and the path as
`channel`. A signal that no model reads, or a `cosim_signal` in a run with no
`cosim_models`, is a rejected stimulus, and the run is invalid rather than a
pass.

## In the run loop

`labwired test` and the browser engine build a `CosimSession` when, and only
when, the manifest declares `cosim_models`. A manifest without them runs the
identical loop it ran before this existed.

Both advance the machine through the session: `CosimSession::advance(machine,
request)` is one lockstep step, and `CosimSession::advance_budget(machine,
request)` repeats it until the request's own fuel or cycle budget is spent.
`labwired test` calls `advance` once per loop iteration; `WasmSimulator`'s
`step`, `step_single`, `step_batch`, `step_batch_profile` and
`step_with_esp32_aids` call `advance_budget`, so `step_batch(n)` still runs `n`
with every model boundary inside it stepped. One `advance`:

1. caps the advance request's **simulated-cycle** budget at the cycles left
   before the next model boundary, so the machine can never run past a boundary
   and hand a model pin levels from its future;
2. advances the machine to that boundary;
3. samples every routed `board.gpio*` path off the bus into the signal store;
4. steps every model whose `step_ns` boundary the machine has reached
   (`CosimRunner::step_until_with_signals`);
5. writes the routed outputs back — GPIO input levels onto pins, volts onto ADC
   channels — so the firmware's next instruction sees the model's answer.

Models are not stepped after a firmware exit or an advance that made no
progress. A routed path that fails at apply time is returned once per distinct
failure rather than once per step.

The lockstep granularity is the finest declared `step_ns`. Simulated time comes
from the machine's own cycle counter (`Machine::total_cycles`) and the bus's
`cpu_hz`, so it is the same clock every trace, `after_cycles` trigger,
`max_cycles` limit and reported `at_cycle` is expressed in. On Cortex-M,
RISC-V and Xtensa that counter advances one cycle per retired instruction. On
the ATmega328P it advances by the core's datasheet cycles (1–4 per step), the
same cycles Timer0 counts, so `millis()` and the circuit agree about time. There are no threads and no
wall clock anywhere in this path: the same firmware produces the same model
inputs on every run.

Multi-node worlds (`labwired test` on an environment, and the browser's
`WasmWorld`) step their nodes without a session, so they refuse to build a node
whose system declares `cosim_models`: "co-simulation models are not supported
in multi-node worlds yet; node '<id>' declares <n>".

A path that does not resolve fails the run at startup instead of degrading it —
a co-simulation whose pin never reached the firmware would otherwise still
print a verdict that is evidence of nothing. A model that errors mid-run ends
the run rather than letting the firmware keep executing against a plant that
stopped answering. Routed outputs are logged under the `cosim` target:

```bash
RUST_LOG=info,cosim=debug labwired test --script examples/cosim-spice-rc/rc-blink.yaml
```

`labwired cosim-step <system.yaml> --set <path>=<value>` still drives a model
directly, without firmware, for probing a manifest.

## In-core analog engine (`adapter: analog`)

The browser cannot spawn a process, so `external_process` — and with it
ngspice — has no counterpart there. `labwired_core::analog` is a small
deterministic MNA transient solver compiled into the engine itself: it runs in
the browser, runs natively, and adds one dependency (`libm`) to either.

A manifest switches engines by changing one line. The `netlist`, `vdd`,
`probes` and `sources` config keys are spelled exactly as the ngspice wrapper
spells them:

```yaml
cosim_models:
  - id: rc_lowpass
    adapter: analog            # was: external_process + model: ./models/rc_lowpass.py
    step_ns: 100000
    inputs:  { gpio: board.gpio.pa5 }
    outputs: { v_out: board.analog.pa0_volts }
    config:
      netlist: ./rc.cir        # or inline: netlist_text: |
      vdd: 3.3                 # volts a boolean input maps to (default 3.3)
      substeps: 10             # internal solver steps per co-sim step
      integration: be          # be (default) | trap
      probes:  { v_out: "v(out)" }   # output name <- node / branch current
      sources: { gpio: Vgpio }       # input name  -> V, I or S element
      trace: ["v(in)", "i(Vgpio)"]   # extra oscilloscope channels
      trace_samples: 20000           # ring depth (default 20000)
```

Worked example: [`examples/cosim-spice-rc/system-analog.yaml`](../examples/cosim-spice-rc/system-analog.yaml),
the same circuit and the same routing as `system.yaml`.

### Netlist subset

| Element | Line |
|---|---|
| Resistor | `R<name> n1 n2 <value>` |
| Capacitor | `C<name> n1 n2 <value> [ic=<v>]` |
| Inductor | `L<name> n1 n2 <value> [ic=<i>]` |
| Voltage source | `V<name> n+ n- <source>` |
| Current source | `I<name> n+ n- <source>` |
| Switch | `S<name> n1 n2 <ctrl> ron=<r> roff=<r>` |
| Diode | `D<name> n+ n- <model>` |
| BJT | `Q<name> nc nb ne <model>` |
| MOSFET | `M<name> nd ng ns nb <model> [w=<m>] [l=<m>]` |

Plus `*` comment lines, `;` / `$` trailing comments, `.end`, and
`.ic V(node)=<v>`. Node `0` and `gnd` are ground. Values take the usual SPICE
suffixes (`k`, `meg`, `m`, `u`, `n`, `p`, `f`, `g`, `t`), and trailing unit
letters are ignored, so `100nF` and `10kohm` read as written. Unlike a classic
SPICE deck, line 1 is NOT a title — put a `*` on it, because silently dropping
an element line is the worst thing a netlist parser can do.

A switch's `<ctrl>` is the name of a routed boolean input, not a circuit node,
so it needs no `sources:` entry.

`.options`, `.tran`, `.op`, `.print`, `.plot`, `.save`, `.probe`, `.width`,
`.temp`, `.nodeset`, `.title` and a whole `.control` … `.endc` block are
accepted and ignored, so one deck can be handed to this engine and to ngspice
unchanged. This engine takes its run length, step and outputs from the
manifest. Any other directive is still a hard error naming the ngspice adapter,
so a typo cannot quietly drop an element line.

### Independent sources

`<source>` is one of:

| Form | Meaning |
|---|---|
| `[dc] <value>` | a constant |
| `SIN(vo va freq [td [theta]])` | `vo + va·exp(−(t−td)·theta)·sin(2π·freq·(t−td))` after `td`, `vo` before |
| `PULSE(v1 v2 [td [tr [tf [pw [per]]]]])` | a trapezoidal pulse train |

Sources are evaluated at the END of each internal step, which is the point the
companion models are written about, and is what SPICE does. The operating point
uses the value at `t = 0`.

A source that carries a function is driven by the clock, so it cannot also be a
routed input: a `sources:` entry pointing at one is a config error rather than
a race between two owners. Give that element a plain `dc` value, or route the
input at a different element.

### Semiconductor models

```text
.model <name> D    (IS=<a> N=<n> RS=<ohms>)
.model <name> NPN  (IS=<a> BF=<n> BR=<n> NF=<n> NR=<n>)
.model <name> PNP  (...)
.model <name> NMOS (VTO=<v> KP=<a/v2> LAMBDA=<1/v> W=<m> L=<m>)
.model <name> PMOS (...)      ; VTO is negative, as in ngspice
```

The parentheses are optional, parameters may be separated by spaces or commas,
and a card may be written after the elements that use it. A parameter omitted
from a card takes **ngspice's** default, so a deck written out in full means the
same thing to both engines.

Parameters this engine has no term for — every capacitance (`CJO`, `CJE`,
`CJC`, `TT`, `CGSO`…), every temperature coefficient, `VAF`, `IKF`, `GAMMA`,
`PHI`, `BV` — are **accepted and ignored**, so a vendor card pasted off a
datasheet runs with the large-signal DC behaviour it describes. A MOSFET
`LEVEL` other than 1 is the one exception and is refused by name: solving a
BSIM card with Shichman–Hodges would be wrong by orders of magnitude rather
than by a capacitance.

Five built-in cards need no `.model` line at all, which is what a catalog part
emits: `D` (1N4148-class, `IS=2.52n N=1.752`), `NPN`, `PNP` (β = 100), `NMOS`,
`PMOS` (`VTO=±1 V`, `KP=20u`, `LAMBDA=0.02`). Those are this engine's
convenience values, not ngspice's parameter defaults.

**What is modelled:** Shockley diode with `RS` and SPICE's `GMIN`;
Ebers–Moll (transport-form Gummel–Poon with `VAF`/`VAR`/`IKF`/`IKR` infinite
and no ohmic terminal resistances); Shichman–Hodges MOSFET level 1 with channel
-length modulation, both channel polarities and reverse mode. Every parameter is
taken at 300.00 K; there is no temperature model, which is why decks handed to
ngspice pin `temp`/`tnom` to 26.85 °C.

**What is not, in this first version:** device capacitances, and therefore
anything whose behaviour comes from stored device charge — reverse recovery, a
Miller-limited edge, a charge pump. There is no Early effect (output conductance
in the active region is `GMIN`, not `Ic/VAF`) and no body effect (`VTH` is
`VTO`; the bulk terminal is required by the syntax and tied through `GMIN`, but
does not shift the threshold). A circuit that needs any of those belongs on
`adapter: external_process` with `tools/cosim/labwired_ngspice.py`.

`RS > 0` on a diode adds one internal node named `<element>#internal`, exactly
as SPICE does. It counts against the 64-unknown ceiling and can be probed.

### Solver

- Modified nodal analysis with companion models. Backward Euler by default;
  `integration: trap` selects trapezoidal, about two orders of magnitude closer
  to the closed form at the same step (0.29 % vs 0.002 % at one tau on the RC
  example) but able to ring on a hard edge. The first internal step after any
  source or switch change is taken with backward Euler, as SPICE does at a
  breakpoint, so an edge does not leave trapezoidal a half-step behind.
- Fixed internal step `h = step_ns / substeps`. Dense LU, own implementation.
  `N` nodes + `M` branch currents is capped at 64; a bigger circuit is an
  error naming the ngspice adapter.
- The solver caches elimination pivots and multipliers until a switch, step
  length, or integration rule changes. Source and reactive-history changes
  refresh only the right-hand side. Multipliers replay in the original order,
  preserving floating-point results. Exactly stationary reactive states reuse
  the previous solution; there is no epsilon cutoff. Every co-simulation
  boundary still samples inputs, routes outputs, and records its waveform row.
  None of these caches adds work to a machine without co-simulation.
- The operating point is solved at t = 0 from the netlist's own DC values,
  capacitors open and inductors shorted, then `.ic` / `ic=` override it. Routed
  inputs apply only once time runs — the same ordering the ngspice wrapper gets
  by pausing its transient just after t = 0, so a pull-up sits at Vdd and a
  GPIO at 0 before the firmware has done anything.
- `CosimStep::time_ns` is the END of the interval being simulated, matching
  `tools/cosim/labwired_ngspice.py`.
- A circuit holding a diode, BJT or MOSFET is solved by Newton–Raphson inside
  each step: the device stamps are re-linearised, re-factorised and re-solved
  until every unknown stops moving, damped by SPICE's `DEVpnjlim`, `DEVfetlim`
  and `DEVlimvds`. The limiters change the path, never the point: the converged
  answer is bit-identical whatever state the iteration started from. Running out
  of iterations is a coded error naming the step, the time and the unknown that
  was still moving — never a `NaN` written into the trace.
- A circuit with **no** nonlinear element takes the pre-Newton code path
  unchanged, down to the floating-point operation order.
  `crates/core/tests/analog_linear_golden.rs` pins that against bit patterns
  captured from the engine before any of this existed.
- Agreement with ngspice on nonlinear decks is measured, not asserted:
  `crates/cli/tests/analog_vs_ngspice_differential.rs` hands the same SPICE
  text to both engines for a half-wave rectifier, a common-emitter amplifier
  and an NMOS inverter, and compares every sample. Measured worst
  disagreement: 2.5e-3 %, 2.6e-5 % and 1.0e-6 % of full scale.
- Deterministic: `f64` only, `Vec` indices in the hot path, `BTreeMap` for
  names, no threads and no wall clock.

### Waveform trace

Every `config.probes` entry is an oscilloscope channel, whether or not the
manifest's `outputs:` routes it anywhere, and `config.trace` adds more. A probe
with no route is how a circuit publishes a node only an instrument looks at:
it is solved and traced every step and never written into the machine.
Samples go into a bounded ring (`config.trace_samples`, default 20 000 — two
seconds at a 100 µs step), read by cursor like `logic_read_edges`:

- core: `Machine::analog_trace_snapshot(cursor)` and `Machine::analog_channels()`,
  after `Machine::attach_analog_trace(...)` with `CosimRunner::analog_trace_registry()`
  or `CosimSession::analog_trace_registry()`.
- WASM: `WasmSimulator::analog_channels()` and
  `WasmSimulator::analog_trace_snapshot(cursor)`. `new_from_config` builds the
  session from the manifest's `cosim_models` and attaches this ring. The
  browser refuses two shapes at construction: `adapter: external_process`
  ("needs a native build; use adapter: analog in the browser") and an analog
  model whose netlist is a file (`netlist:`; put it inline as `netlist_text`).
  `mock` and `analog` with `netlist_text` run as they do natively.
- CLI: `--analog-trace <path>` on `run`, `test` and `cosim-step`. A `.csv`
  extension writes `time_ns,<channel>...`; anything else writes a VCD with one
  `real` variable per channel, so the analog curve opens in GTKWave / PulseView
  beside the digital logic capture. `labwired test` attaches its co-simulation
  session's ring, so the file holds the waveform of the firmware-driven run:

  ```bash
  labwired test --firmware tests/fixtures/stm32f401-blinky.elf \
      --system examples/cosim-spice-rc/system-analog.yaml \
      --max-steps 1500000 --analog-trace rc.csv
  ```

  `labwired run` steps no co-simulation models, so there the file has channels
  only when something else attached a runner; `cosim-step` writes its own
  runner's ring.

All analog models on one runner share one ring, each owning a block of
channels; a model that steps writes a full row and carries the other models'
channels forward, which is what a scope shows between updates. Every channel is
named `<model id>.<name>` (the probe name, or the `trace:` expression), however
many analog models are declared. A consumer can then compute a channel name
from the manifest alone, and adding a second circuit does not rename the first
circuit's channels. The CSV header and VCD variables use the same names.

### The boundary

The in-core engine is linear elements and ideal switches, and nothing else. It
does not model diodes, transistors, subcircuits, `.include` / `.lib` device
libraries, or AC/DC sweeps, and it does not approximate them: a netlist line
outside the subset fails **manifest validation** with

```
element `D1 a b diode` needs ngspice; use `adapter: external_process` with `tools/cosim/labwired_ngspice.py`
```

That is the whole boundary. Native runs that need real device physics use the
ngspice adapter above, which has none of these limits; the browser runs the
in-core engine, which needs no process.
