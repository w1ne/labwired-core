# LabWired Audit Protocol (Golden Reference)

The `labwired-audit` tool is the automated bridge for our **"Hardware Oracle"** claim. it produces a verifiable evidence package proving that the LabWired simulator matches physical hardware behavior bit-for-bit.

## 🛠️ The `labwired-audit` Tool

### CLI Interface
```bash
./scripts/labwired-audit.py \
  --hw-trace out/golden-reference/hw_trace.json \
  --sim-trace out/golden-reference/sim_trace.json \
  --target "NUCLEO-H563ZI" \
  --firmware "firmware-h563-demo" \
  --output out/golden-reference/determinism_report.json
```

### Input Requirements
1.  **Hardware Trace (`hw_trace.json`)**: Captured from a physical board using a hardware debugger (e.g., AetherDebugger/probe-rs).
2.  **Simulator Trace (`sim_trace.json`)**: Captured from LabWired using `--trace` mode.

### Comparison Logic
The tool executes the following verification steps:
1.  **Firmware Integrity**: Verifies SHA-256 hashes of the binaries used in both traces.
2.  **Architectural Parity**:
    - **PC Match**: Every retired instruction address must match.
    - **Register Match**: (Phase 2) General-purpose registers must match at synchronization points.
    - **UART Flow**: Output bytes must be identical in sequence and value.
3.  **Synchronization**: Automatically identifies the Reset-to-Entry transition to align trace start points.

### Output: Determinism Report
Produces the standard `determinism_report.json` which includes:
- Summary status (`PASS`/`FAIL`).
- Step-by-step discrepancy log (if any).
- Cryptographic signature of the run (future-state).

## 🚀 Execution Workflow

1.  **Capture HW**: User (or CI) runs firmware on real board; Aether collects the trace.
2.  **Capture SIM**: `labwired --firmware <elf> --trace sim_trace.json`.
3.  **Audit**: `labwired-audit` generates the report.
4.  **Publish**: The report is attached to the release/commit as evidence.
