# One shared analog circuit in a world

An environment can own one in-core analog circuit. Each controller keeps its own
CPU, bus, pins, and ADC state. The circuit uses one MNA solver and one world
timeline. This supports electrically coupled controllers without cloning the
power circuit or aliasing equal pad names from different chips.

## Manifest and route ownership

Declare the model on the environment, alongside `nodes` and `interconnects`:

```yaml
name: two-controllers
nodes:
  - { id: a, system: a.yaml, firmware: a.elf }
  - { id: b, system: b.yaml, firmware: b.elf }
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 1000
    inputs:
      drv_a: node.a.board.gpio_pad_volts.pa8
      ctl_a: node.a.board.gpio_driven.pa8
      drv_b: node.b.board.gpio_pad_volts.pa8
      ctl_b: node.b.board.gpio_driven.pa8
    outputs:
      sense_a: node.a.adc.adc1.0_volts
      sense_b: node.b.adc.adc1.0_volts
    config:
      netlist_text: |
        Va drive_a 0 dc 0
        Vb drive_b 0 dc 0
        Sa drive_a a ctl_a ron=25 roff=1e12
        Sb drive_b b ctl_b ron=25 roff=1e12
        Ra a shared 1k
        Rb b shared 1k
        Rload shared 0 1k
        .end
      sources: { drv_a: Va, drv_b: Vb }
      probes: { sense_a: 'v(a)', sense_b: 'v(b)', shared: 'v(shared)' }
```

The two system manifests must not declare `cosim_models`. A per-node model is
still refused, including when the environment also declares a shared model.
The bounded contract accepts exactly one `adapter: analog` model with inline
`config.netlist_text`. File decks, external-process adapters, multiple models,
and simultaneous BLE scheduling are refused. Worlds with no shared model keep
the existing execution paths.

Every hardware route starts with `node.<id>.`; node IDs use 1–32 ASCII letters,
digits, underscores, or hyphens. The remainder uses the existing board/ADC
route grammar. `ui.*` can supply external model inputs. Unknown nodes, unresolved
hardware, wrong route direction, multiple writers to the same resolved sink,
and missing output probes fail construction. An output alias such as
`board.analog.pa0_volts` does not evade ownership of `adc.adc1.0_volts`.
A GPIO endpoint cannot belong to both a digital `gpio_net` and the analog
router. Disjoint digital nets can coexist.

The model-local names (`drv_a`, `sense_a`) and SPICE names (`Va`, `a`) are not
hardware identities. The qualified route establishes the owning controller.
One solved node may feed several distinct observations, including ADC and
comparator channels when those peripheral models accept the declared channels.
The generic tests exercise real ADC conversion; qualification of a specific
comparator belongs to that chip's integration tests.

## Sampled coupling and time

The runtime uses **interval-start zero-order hold**:

1. Read all controllers' model inputs at the start of the interval.
2. Hold those values while each CPU reaches the interval boundary.
3. Integrate the single shared circuit to that boundary once.
4. Apply all solved outputs before any controller advances into the next
   interval. Within each controller, VDD is applied before ADC reference and
   reset input.

A CPU reaches the first instruction boundary at or after the requested time.
A GPIO edge inside an interval affects the next interval; it does not change
the circuit's past. Smaller `step_ns` reduces this sampling error but raises
cost. Solver substeps improve circuit integration with the held inputs; they
do not recover unobserved firmware edges. No universal PWM/DShot interval or
flight-accuracy claim follows from this mechanism.

Initialization reads the netlist's solved operating point at time zero before
CPU execution. It adds no integration step and no duplicate time-zero trace
row. Reactive state therefore starts from the declared operating point; model
an explicit supply transition when a cold power-on ramp is required.

`World::cosim_status()` and `WasmWorld.cosim_status()` report the model ID,
`step_ns`, completed time, next boundary, step count, coupling mode, failures,
and per-node reset state. The existing analog channel, trace-snapshot, and
series methods expose the same circuit in native and Wasm worlds. CLI
environment results include `cosim` and `analog_series`; `--analog-trace` exports
the world series through the existing artifact writer. A native Wasm wrapper
test covers transport and timestamps; it does not establish browser rendering.
`run_until_ps` and `step_rounds` use the same analog-aware round path, including
worlds that also declare digital GPIO nets. GPIO batching cannot skip circuit
steps, and an analog-only world exposes its round clock to these APIs.
External `ui.*` values can change only at completed analog boundaries. A solver,
route, or CPU failure latches the circuit failure and blocks further stepping.

## Driver voltage and high impedance

`board.gpio_pad_volts.<pad>` reads the live pad and returns a numeric driver
voltage. Its high amplitude follows that controller's routed VDD. With no
routed supply it uses the chip's declared `io_voltage_v`. It is zero while VDD
or the external reset input holds the controller in reset.

Keep direction separate: `board.gpio_driven.<pad>` opens the circuit's driver
switch when the MCU output is high impedance. It is also false while reset is
held. Different controllers can therefore have the same pad name and different
rails without sharing a driver or forcing a released pin to zero volts.
The netlist still owns output resistance, leakage, protection, and other pad
physics; this route does not invent them. GPIO input Schmitt thresholds still
use nominal descriptor VDD in this patch. The existing VDDA router also refuses
a zero-volt ADC reference. A separate power-routing follow-up must address both
before validating a variable-rail or fully powered-off native circuit.

## Descriptor-owned external reset

`board.reset.<pin>_volts` is a write-only electrical input, not a GPIO alias.
Binding requires a matching `reset_input` declaration on the chip. A descriptor
can use one absolute threshold pair or one ratio pair:

```yaml
reset_input:
  pin: NRST
  active_low: true
  vil_max_v: 0.72
  vih_min_v: 2.0
  min_assert_ns: 80000
  invalid_pulse_ns: 40000
  pull_up_ohms: 40000
```

These example values come from the [AT32F421 datasheet, Ver 2.03, Table 35](https://www.arterychip.com/download/DS/DS_AT32F421_V2.03_EN.pdf),
printed page 46. They are absolute 0.72 V / 2.0 V limits, not fractions of the
nominal rail. The source PDF SHA-256 is
`953409957662e1fe4fada766411913ca832382551500f20fad474dec0739c0de`.
The generic regression fixture applies these explicit limits to a shipped
F401 bus to test routing; it does not assert that they are F401 silicon limits.

Alternatively use `vil` / `vih`, with `0 < vil < vih < 1`, for a chip whose
source specifies VDD ratios. Ratio thresholds use the same boundary's actual
routed VDD. Mixed or incomplete threshold pairs, reversed limits, non-finite
values, and a non-positive `pull_up_ohms` fail parsing. The lowerer stamps the
optional internal resistor to that MCU's actual VDD node; reset state does not
substitute an ideal rail.

A routed reset initially holds execution until a known level arrives. Attaching
it, or observing an initial voltage in the Schmitt band, does not create an
external reset cause. A qualified pulse calls the existing `reset_node` hook
once. A pulse at or below `invalid_pulse_ns` is ignored. A pulse at or above
`min_assert_ns` is valid, including when its release is the next sampled
observation. A pulse strictly between the limits fails as indeterminate.
Pulse widths and crossings are sampled at `step_ns`; narrower unobserved pulses
are outside this coupling contract.

VDD hold and external reset hold are independent. CPU execution resumes only
when both release. Reset uses each peripheral's existing reset hook and retains
RAM according to that hook; it does not claim every unqualified peripheral has
silicon-complete reset behavior. The AT32 datasheet's typical 4.5 ms power-on
release delay is not introduced by this generic change.

## Regression evidence

`crates/core/tests/world_analog.rs` builds real ARM nodes and a single MNA deck.
It checks equal-named pin independence, shared-rail load coupling and KCL,
independent ADC conversions, high impedance, distinct nominal and routed VDD,
interval-start sampling, deterministic node order and host chunking, reset RC
hold/release, combined supply/reset hold, and route refusal. The reset tests
cover absolute and ratio thresholds, valid/invalid/indeterminate pulses,
initial unknown levels, and release-only qualification.

`crates/config/tests/world_cosim.rs` covers the manifest and reset descriptor
contracts. `crates/wasm/src/world.rs` loads the same core path through resolved
node transport and checks distinct divider observations and exact trace times.
The validation receipt in `docs/evidence/world-analog-20261112.json` records
commands, exit status, and the scope of these checks.
