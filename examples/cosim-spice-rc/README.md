# Co-Simulation with ngspice — RC low-pass

Mixed-signal co-simulation through LabWired's `external_process` contract:
`tools/cosim/labwired_ngspice.py` hosts libngspice in-process and steps a
SPICE netlist in lockstep with the firmware clock. A firmware GPIO edge becomes
a voltage-source change; a probed node voltage comes back each step.

The netlist here is a 10 kΩ / 100 nF low-pass (τ = 1 ms). Any ngspice-compatible
model library works the same way — `.include` / `.lib` vendor or open-source
models in the `.cir` file and map the sources and nodes you care about.

## Run it with firmware

```bash
sudo apt install libngspice0          # once
RUST_LOG=info,cosim=debug labwired test --script examples/cosim-spice-rc/rc-blink.yaml
```

That runs the committed NUCLEO-F401RE Arduino blink image against this
manifest. Nothing injects a value: the firmware drives LD2 (PA5), the manifest
routes that pad into the netlist's `Vgpio` source, and the probed node voltage
comes back on ADC1 channel 0 — the channel PA0 belongs to — every 100 µs. The
`cosim` log lines are the waveform:

```text
board.gpio.pa5 = true (cycle=8400) -> models
rc_lowpass -> board.analog.pa0_volts = 0.31670723152394775 (cycle=8400)
rc_lowpass -> board.analog.pa0_volts = 0.6006056365027725 (cycle=16800)
...
rc_lowpass -> board.analog.pa0_volts = 2.0870859484385904 (cycle=84000)
...
rc_lowpass -> board.analog.pa0_volts = 3.2999999998479335 (cycle=1999200)
```

At 84 MHz one 100 µs step is 8400 cycles, so cycle 84000 is 1 ms — one time
constant — and the node is at 3.3 V × (1 − e⁻¹) ≈ 2.09 V, as it should be. The
sketch drives LD2 high at cycle 6072 (confirm with `--watch-gpio gpioa:5`), so
the first boundary already sees it.

The machine is capped at each model boundary, so the circuit never sees a pin
level from the firmware's future — see
[Board signal paths](../../docs/cosimulation_plugins.md#board-signal-paths).

`rc-blink.yaml` is not named `test*.yaml` on purpose: `scripts/example_smokes.sh`
would then run it on machines without libngspice0 and report the machine rather
than the example.

## Probe it without firmware

```bash
labwired cosim-step examples/cosim-spice-rc/system.yaml \
    --set board.gpio.pa5=true
```

`cosim-step` builds the runner from the manifest, feeds `board.gpio.pa5` into the
model as the `gpio` source (booleans map to Vdd / 0 V), steps the circuit to the
model's `step_ns` boundary and prints `board.analog.pa0_volts`.

Standalone, without LabWired:

```bash
printf '{"time_ns":1000000,"dt_ns":1000000,"inputs":{"gpio":true}}\n' \
  | python3 examples/cosim-spice-rc/models/rc_lowpass.py
# {"outputs": {"v_out": 2.08...}}      (3.3 V × (1 − e⁻¹) after one τ)
```

## Contract details

- Inputs: `true`/`false` → `vdd` / 0 V; numbers → volts directly.
- Outputs: the probed vector's last value at `time_ns`, the step's end time.
- Determinism: no threads or wall clock; the operating point is solved from
  the netlist's own defaults, then inputs are applied once time is running.
- One circuit per wrapper process (libngspice is process-global). Declare a
  second `cosim_models` entry for a second circuit.
- `board.gpio.<pad>` reads what the firmware drives; `board.analog.<pad>_volts`
  writes the ADC input the chip descriptor's `analog_pins:` assigns to that
  pad. The full path grammar, and the explicit
  `adc.<peripheral>.<channel>_volts` form for chips whose descriptor names no
  analog pads, are in
  [`docs/cosimulation_plugins.md`](../../docs/cosimulation_plugins.md#board-signal-paths).

Tests: `python3 -m pytest tools/cosim/test_labwired_ngspice.py`.

## The same circuit without ngspice

`system-analog.yaml` declares the same netlist, the same routing and the same
probe against `adapter: analog` — the in-core MNA engine
(`labwired_core::analog`). It spawns no process, so it is the variant the
browser can run:

```bash
labwired cosim-step examples/cosim-spice-rc/system-analog.yaml \
    --set board.gpio.pa5=true --steps 20 --analog-trace /tmp/rc.csv
```

The two engines are held to within 1 % of each other at every step by
`crates/cli/tests/analog_vs_ngspice_differential.rs`. What the in-core engine
does not do — diodes, transistors, `.include` model libraries, AC/DC sweeps —
it refuses by name and points back at the ngspice adapter above; see
`docs/cosimulation_plugins.md`.
