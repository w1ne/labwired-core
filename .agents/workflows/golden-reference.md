---
description: How to generate and verify a Golden Reference (Determinism Report)
---

This workflow guides you through producing a verifiable proof of hardware-simulation parity.

### 1) Capture Hardware Trace
Use your hardware debugger (e.g., AetherDebugger) to capture an instruction trace from a real board.
Save it to `out/golden-reference/hw_trace.json`.

### 2) Capture Simulator Trace
Run the same firmware in the LabWired simulator with the `--trace` flag.

// turbo
```bash
cd core
cargo run --release -p labwired-cli -- \
    --firmware path/to/demo_firmware.elf \
    --system configs/systems/nucleo-h563zi-demo.yaml \
    --trace out/golden-reference/sim_trace.json \
    --max-steps 1000
```

### 3) Run Audit Tool
Compare the two traces to generate the Determinism Report.

// turbo
```bash
cd core
./scripts/labwired-audit.py \
    --hw-trace out/golden-reference/hw_trace.json \
    --sim-trace out/golden-reference/sim_trace.json \
    --target "NUCLEO-H563ZI" \
    --firmware "firmware-h563-demo" \
    --output out/golden-reference/determinism_report.json
```

### 4) Verify Status
Ensure the status is `PASS` in the generated `determinism_report.json`.

```bash
grep '"status": "PASS"' out/golden-reference/determinism_report.json
```
