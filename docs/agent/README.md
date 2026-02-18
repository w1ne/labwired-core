# LabWired AIPi: Technical Specification for Agents & Humans

LabWired is a **deterministic hardware oracle**. This document specifies the interfaces for interacting with the simulator programmatically (via agents) or manually.

## 1. MCU Onboarding Policy (Config-Only by Default)

For new MCU/board onboarding, the default path is **configuration-only**:

1. Add chip config in `core/configs/chips/<chip>.yaml`.
2. Add system config in `core/configs/systems/<board>.yaml`.
3. Add/update `board_io` mapping when board-level IO should be visualized.
4. Validate using existing runner flows (`labwired test` / CLI), without creating board-specific engine code by default.

If code changes are required, treat that as a separate "engine enablement" change with explicit tests and then continue onboarding via config.

## 2. The Agentic "Iterative Loop" Protocol

LabWired is designed for **autonomous refinement**. External agents should follow this iterative protocol when generating new peripheral models:

1.  **Hypothesize**: Extract an initial `IrPeripheral` model from a datasheet (PDF/HTML).
2.  **Simulate**: Load the model into the LabWired sandbox using the `labwired` Python module.
3.  **Verify**: Apply stimulus (writes) and check responses (reads) using `labwired_ai.executor`.
4.  **Audit**: Compare the simulation behavior against `HardwareRules` in `labwired_ai.rules`.
5.  **Fix**: If a deviation is found (e.g., wrong reset value), the agent updates the source IR and repeats from Step 2.

This protocol ensures that intelligence remains in the agent realm while the core provides the bit-accurate sandbox.

## 3. CLI Interface

The `labwired` binary provides several execution modes:

### Interactive Mode (Default)
Runs a firmware ELF in the simulator.
```bash
labwired --firmware <ELF> [--system <YAML>] [--vcd <PATH>] [--json]
```
- `--firmware`: Path to the ELF to execute.
- `--system`: Path to `system.yaml` (optional, defaults to Cortex-M).
- `--vcd`: Export simulation trace to VCD file.
- `--json`: Output structured performance/error data instead of logs.

### Asset Foundry Commands
```bash
labwired asset init -o <DIR> [--chip <CHIP>]
labwired asset import-svd -i <SVD> -o <JSON>
labwired asset codegen -i <JSON> -o <RUST>
labwired asset validate --system <YAML>
labwired asset list-chips --json
```

## 4. Agent Validation Tools (`asset validate`)

Agents constructing `system.yaml` or `chip.yaml` files programmatically can verify their integrity using the `validate` command.

### Validation Result Schema

All validation commands output structured JSON with the following schema:

```json
{
  "valid": boolean,
  "issues": [
    {
      "severity": "error" | "warning" | "info",
      "code": string,
      "message": string,
      "suggestion": string | null,
      "location": string | null
    }
  ],
  "context": string,
  "statistics": {
    "total_checks": number,
    "errors": number,
    "warnings": number,
    "infos": number
  }
}
```

### Discovery (`list-chips`)
Agents can discover available supported hardware platforms:
```bash
labwired asset list-chips --json
```

## 5. Metering API (`result.json`)

To enable the agent economy, the simulator provides high-precision telemetry for billing and quota management. These metrics are always available in the `result.json` output when running `labwired test`.

### Schema

```json
{
  "result_schema_version": "1.0",
  "status": "pass" | "fail",
  "steps_executed": number,
  "cycles": number,
  "instructions": number,
  "stop_reason": StopReason,
  "stop_reason_details": {
    "triggered_stop_condition": StopReason,
    "triggered_limit": { "name": string, "value": number } | null,
    "observed": { "name": string, "value": number } | null
  },
  "limits": {
    "max_steps": number,
    "max_cycles": number | null,
    "max_uart_bytes": number | null,
    "no_progress_steps": number | null,
    "wall_time_ms": number | null,
    "max_vcd_bytes": number | null
  },
  "message": string | null,
  "assertions": [{ "assertion": TestAssertion, "passed": boolean }],
  "firmware_hash": string,
  "config": {
    "firmware": string,
    "system": string | null,
    "script": string
  }
}
```

### Quota Management

Agents should budget based on **Instructions**, as this is deterministic and independent of the specific peripheral latency models which may change between simulator versions.

- **Standard Run Limit**: 1,000,000 Instructions (~10ms real-time @ 100MHz).
- **Hard Limit**: 100,000,000 Instructions.

### Programmatic Access Example

```python
import json
from pathlib import Path

def run_simulation(test_script, output_dir="out"):
    subprocess.run(["labwired", "test", "--script", test_script, "--output-dir", output_dir], check=True)
    return json.load(open(Path(output_dir) / "result.json"))

result = run_simulation("test.yaml")
print(f"Cycles: {result['cycles']:,}, Status: {result['status']}")
```

## 6. VCD Trace Export

LabWired can export simulation traces to VCD (Value Change Dump) format for analysis with standard logic analyzer tools like GTKWave.

### Usage

```bash
labwired --firmware firmware.elf --vcd trace.vcd
```

### VCD Format & Signal Hierarchy

**Standard**: IEEE 1364-2001 compliant
**Timescale**: 1 nanosecond per step

- `top` (module)
  - `pc` [31:0]: Current Program Counter (updates at step start).
  - `bus` (module)
    - `addr` [31:0]: Target address of the last write.
    - `data` [7:0]: Data byte of the last write.
    - `we` [0:0]: Write Enable strobe (pulsed high during memory writes, resets at next step).

## 7. Unsupported Instruction Audit

Agents should run a firmware execution audit to discover decoder/executor gaps from real code paths.

Run from repository root:

```bash
core/scripts/unsupported_instruction_audit.sh \
  --firmware core/target/thumbv7m-none-eabi/release/<firmware-demo-crate> \
  --system core/configs/systems/<board>.yaml \
  --max-steps 200000 \
  --out-dir core/out/unsupported-audit/<board>
```

When `--json` is enabled, the CLI suppresses standard logging and emits machine-readable events. Use the generated `report.md` and `*.tsv` files as the implementation backlog.

## 8. Error Schemas

All CLI commands support the `--json` flag for machine-readable error output.

### Error Response Format
```json
{
  "error_type": "ConfigError",
  "message": "Failed to parse system manifest: ...",
  "details": { ... },
  "exit_code": 2
}
```

### Exit Codes
- **0**: Success (pass)
- **1**: Assertion failure (test mode only)
- **2**: Configuration error
- **3**: Runtime error

## 9. SVD Ingestion Pipeline (Grounding)

LabWired allows agents to "ground" their knowledge by ingesting vendor-standard CMSIS-SVD files. This is the preferred method for generating high-fidelity `PeripheralDescriptor` YAMLs.

### Workflow
1.  **Acquire**: Agent locates `.svd` file for the target chip.
2.  **Ingest**: Run `svd-ingestor` to generate canonical YAMLs.
3.  **Integrate**: Reference generated YAMLs in `system.yaml`.

### Command
```bash
cargo run -p svd-ingestor -- \
  --input <PATH_TO_SVD> \
  --output-dir <OUTPUT_DIRECTORY> \
  --filter <PERIPHERAL_NAMES>
```

### Example
```bash
# Generate descriptors for RCC and USART2
cargo run -p svd-ingestor -- \
  --input core/tests/fixtures/real_world/stm32f401.svd \
  --output-dir core/examples/my_board/peripherals \
  --filter RCC,USART2
```

## 10. Agentic Toolset (Python)
## 11. Documentation Index

For detailed agent workflows and architecture, refer to:
- [Architecture Guide](./architecture.md)
- [Hardware Onboarding (SVD & AI)](./workflows/hardware_onboarding.md)
- [Roadmap](./roadmap.md)

### Workflows
- [Instruction Audit](./workflows/unsupported_instruction_audit.md)

The `ai/labwired_ai` directory provides the high-level toolset for agents following the Iterative Loop protocol.

### Stimulus-Response (`executor.py`)
Agents use `AgenticExecutor` to poke the simulation without writing real C/Rust firmware.

```python
from labwired_ai.executor import AgenticExecutor

exec = AgenticExecutor()
# Trigger: Write 0x01 to control register
# Expect: Status bit 0 becomes high after 100 cycles
result = exec.verify_behavior(
    trigger_op={"op": "write", "addr": 0x40001000, "val": 0x01},
    expected_response={"op": "read", "addr": 0x40001004, "val": 0x01},
    timeout_cycles=100
)
print(f"Correctness: {result['success']}")
```

### Self-Correction Rules (`rules.py`)
Formalizes the "Hardware Rules" that a model must satisfy to be considered valid.

```bash
python3 -m labwired_ai.rules --model my_peripheral.yaml --device generic_uart
```
