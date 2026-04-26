# Golden Reference: Hardware vs. Simulation Parity

LabWired's defining property is *deterministic parity with real hardware*. To back that up, the project runs a two-phase pipeline that captures execution from a real board and diffs it against the simulator running the same firmware ELF. This document describes the pipeline, the published evidence, and how to reproduce it.

## The published proof

For NUCLEO-H563ZI, the following artifacts are committed under [`examples/nucleo-h563zi/golden-reference/`](../examples/nucleo-h563zi/golden-reference/):

| File | What it is |
| --- | --- |
| `hw_trace.json` | Instruction-level trace from the real board (OpenOCD + GDB-MI). |
| `sim_trace.json` | Instruction-level trace from `labwired` running the same firmware. |
| `determinism_report_h563.json` | Aggregate audit verdict. Current published run: `status: PASS`, 50 compared steps. |
| `result.json` | Simulator's structured run result (status, steps, cycles, stop reason). |
| `snapshot.json`, `uart.log`, `junit.xml` | Supporting artifacts (final architectural state, UART output, CI report). |

Verifying the published report is a one-line read:

```bash
jq '{status, steps_compared, matches: (.results | length)}' \
    examples/nucleo-h563zi/golden-reference/determinism_report_h563.json
```

## Phase 1 — capture from hardware

[`scripts/capture_hardware_trace.py`](../scripts/capture_hardware_trace.py) drives a connected NUCLEO-H563ZI via OpenOCD + GDB-MI. For N steps it:

1. Starts OpenOCD with the `stlink-dap` interface and `stm32h5x` target config.
2. Loads the firmware ELF onto the board.
3. Halts at reset.
4. For each step: reads all registers, captures the PC, and single-steps the CPU.
5. Emits `hw_trace.json` as a JSON array of `{step, pc, registers}` records.

Required tooling on the host: `openocd`, `gdb-multiarch`, `pygdbmi`. The script's defaults assume the demo firmware ELF and system YAML — adjust the constants at the top to retarget.

## Phase 2 — audit against the simulator

[`scripts/repro_golden_reference.sh`](../scripts/repro_golden_reference.sh) takes a captured `hw_trace.json` and:

1. Runs the simulator with the same firmware + system YAML, capturing `sim_trace.json`.
2. Invokes [`scripts/labwired-audit.py`](../scripts/labwired-audit.py), which:
   - Aligns the two traces by first-common-PC (the real board executes a few cycles of bootloader stub before reaching the firmware entry, which the simulator does not model).
   - Walks both traces step-by-step, comparing PCs.
   - Emits `determinism_report.json` with per-step match/mismatch and an aggregate `status`.

Reproducing the published H563 run:

```bash
./scripts/repro_golden_reference.sh \
    examples/nucleo-h563zi/golden-reference/hw_trace.json \
    target/thumbv7em-none-eabihf/release/firmware-h563-demo \
    configs/systems/nucleo-h563zi-demo.yaml \
    NUCLEO-H563ZI
```

The audit report lands in `out/golden-reference/`. Compare it against the committed `examples/nucleo-h563zi/golden-reference/determinism_report_h563.json`.

## Why this matters

A static "we simulate Cortex-M33" claim is a wishlist. A captured hardware trace, a simulator trace, a diff, and a committed report is evidence.

The pipeline answers the first question every embedded engineer asks before trusting a simulator: *does it actually match what the chip does?* Architectural state divergence between sim and hardware is the bug class that ruins firmware development workflows — peripherals that "work in the sim" but lock up the chip, race conditions that only appear on metal, and so on. The golden-reference workflow turns "trust me" into a CI artifact.

## Extending to a new board

The capture script is hardcoded to H563 paths today. For a new board:

1. Build the smoke firmware for the new chip; record the ELF path.
2. Confirm OpenOCD has a target config (`target/<chip>.cfg`) for your debug probe.
3. Copy `capture_hardware_trace.py`, edit the `OPENOCD_ARGS`, `FIRMWARE_ELF`, and `SYSTEM_YAML` constants.
4. Capture, audit, commit the artifacts under `examples/<board>/golden-reference/`.

A future iteration would parameterize the capture script over CLI args. For now it is intentionally simple — the trace data is the artifact, not the script.
