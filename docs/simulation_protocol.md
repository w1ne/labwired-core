# LabWired Simulation Protocol

## Overview

The LabWired Simulation Protocol defines the stable, versioned contract between the core simulation engine and external integrations (CI pipelines, agentic workflows, custom tooling). Unlike interactive debugger protocols like DAP or GDB RSP, which target live human inspection, the **Simulation Protocol** focuses on determinism, machine-readable inputs/outputs, and reproducible verification.

This specification documents the v1.0 schema for inputs, expected event lifecycles, and output artifacts.

---

## 1. Simulation Lifecycle

The lifecycle of a headless simulation run is strictly defined as follows:

1.  **Configuration**: The runner parses the test script, system manifest, and hardware models.
2.  **Reset**: The CPU and all peripherals are initialized. The firmware image is loaded into memory.
3.  **Execution Loop**:
    - The CPU fetches and executes an instruction.
    - The system bus ticks all peripherals, incrementing cycle counts.
    - Limits and assertions are evaluated.
4.  **Halt Condition**: The simulation stops when one of the terminal conditions (limits, assertions, or exceptions) evaluates to true.
5.  **Artifact Emission**: When `--output-dir` is supplied, the runner writes
    `result.json`, `snapshot.json`, `uart.log`, and JUnit XML there;
    `--junit` can request JUnit at a separate path. Requested traces are
    flushed before the runner exits with a standardized code.

---

## 2. Input Manifests (v1.0)

LabWired relies on declarative configuration files to define the "digital twin" of your hardware.

### 2.1 Test Script (`test_script.yaml`)

The test script dictates the bounds and expectations of the simulation. Its
`inputs` object has one mutually exclusive v1.0 form: a single-machine
`firmware`/`system` pair or an environment `env` reference.

```yaml
schema_version: "1.0"
inputs:
  firmware: "path/to/firmware.elf"
  system: "path/to/system.yaml"      # Optional: default board is used if omitted
limits:
  max_steps: 100000000               # Required: Hard limit on CPU instructions
  max_cycles: 150000000              # Optional: Limit on core clock cycles
  wall_time_ms: 10000                # Optional: Timeout in real-world wall clock milliseconds
  max_uart_bytes: 512000             # Optional: Limit on total UART characters emitted
  no_progress_steps: 50000           # Optional: Halt if PC remains unchanged
assertions:
  - expected_stop_reason: halt       # Halt instruction (e.g., BKPT, WFI loop)
  - uart_contains: "TEST PASSED"     # Substring match on the UART output stream
  - memory_value:                    # Assert value at specific address
      address: 0x40000030
      expected_value: 0x80
      size: 8                        # Optional: 8, 16, or 32 (default: 32)
      mask: 0x80                     # Optional: bitmask for comparison
```

For a multi-node world, use `inputs.env`; topology and per-node firmware stay
out of the test script:

```yaml
schema_version: "1.0"
inputs:
  env: "two-node-env.yaml"
limits:
  max_steps: 100000
assertions:
  - memory_value:
      node: gateway
      address: 0x20000000
      expected_value: 0
      size: 8
```

Environment assertions are node-qualified `memory_value` assertions. CLI
`--firmware` and `--system` overrides apply only to a single-machine script,
not to `inputs.env`. v0.19 environment worlds are Cortex-M-only: each node's
system/chip and firmware ELF must be ARM/Cortex-M. A non-Arm system/chip or
firmware ELF is rejected with a configuration error until architecture-specific
world setup exists.

### 2.2 System Manifest (`system.yaml`)

Defines the board topology, Memory Management overrides, and IO mappings. This
same per-node `system.yaml` is shared with the Playground; CI environment
manifests reference it rather than duplicating board configuration.

```yaml
schema_version: "1.0"
name: "nucleo-f401re"
chip: "stm32f401.yaml"
memory_overrides:
  flash: "512KB"
board_io:
  - id: "USER_LED"
    kind: "led"
    peripheral: "GPIOA"
    pin: 5
    active_high: true
```

### 2.3 Environment Manifest (`environment.yaml`)

An environment manifest is a released v1.0 world description. Each node names
the system manifest and firmware it runs; all topology is explicit in
`interconnects`.

```yaml
schema_version: "1.0"
name: "mesh-network-test"
nodes:
  - id: "gateway"
    system: "gateway.yaml"
    firmware: "gateway.elf"
  - id: "sensor-01"
    system: "sensor.yaml"
    firmware: "sensor-01.elf"
interconnects:
  - type: "can_bus"
    nodes: ["gateway", "sensor-01"]
    config:
      peripheral: "can1"
```

`nodes[].config_overrides` is rejected in environment schema 1.0. Interconnect
membership is strict:

- `can_bus` requires a nonblank `config.peripheral` and at least two unique,
  known nodes.
- `uart_cross_link` requires exactly two unique, known nodes.
- `egress` requires exactly one known node.

### 2.4 Hardware Descriptors (`chip.yaml` & `peripheral.yaml`)

For peripheral definitions, refer to the [schema compatibility guide](./schema_compatibility.md). LabWired supports strict IR definitions encompassing registers, fields, reset values, and side-effects (`WriteOneToClear`).

---

## 3. Event Taxonomy

While the simulation executes deterministically, it emits a standardized stream of events that can be asserted against or captured in output traces.

### 3.1 UART Events
- **Emission**: Occurs when a simulated CPU writes to a memory-mapped UART Transmission Data Register (TDR).
- **Semantics**: Aggregated into a continuous stream per UART instance. Standard assertions (`uart_contains`, `uart_regex`) operate on this stream.

### 3.2 GPIO Events
- **Emission**: Occurs when a GPIO output data register or bit set/reset register is modified.
- **Semantics**: Captured as discrete rising/falling edges in the VCD trace. In future v1.1, GPIO events will support routing to external FMUs (Functional Mock-up Units) for Hardware-in-the-Loop simulation via FMI 3.0.

### 3.3 Interrupt Requests (IRQ)
- **Emission**: Occurs when a peripheral asserts an interrupt line to the NVIC (Nested Vectored Interrupt Controller) or equivalent core interrupt controller.
- **Semantics**: Traces capture both the assertion of the IRQ line by the peripheral and the subsequent entry into the Exception Handler by the CPU.

### 3.4 Faults and Exceptions
- **Emission**: Generated synchronously by the CPU when an illegal operation is attempted (e.g., HardFault, MemManage, UsageFault).
- **Semantics**: Faults transition execution to a fault handler. If the fault is unrecoverable (e.g., branching to unmapped memory), it triggers a `MemoryViolation` or `DecodeError` stop condition.

---

## 4. Output Artifacts

Upon completion of a deterministic run, LabWired produces a bundle of artifacts.

### 4.1 Structured Summary (`result.json`)

Provides programmatic access to the simulation's final state and metrics. The
common fields are `status`, `steps_executed`, `cycles`, `instructions`,
`stop_reason`, `stop_reason_details`, `limits`, and `assertions`.
`result.json` uses the same top-level `oneOf` union as the
[CI Test Runner](ci_test_runner.md#resultjson-contract). The abbreviated schema
below highlights its discriminators and provenance; the CI Test Runner page is
the authoritative complete schema:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "oneOf": [
    {
      "title": "single-machine result",
      "type": "object",
      "required": ["result_schema_version", "firmware_hash", "config"],
      "properties": {
        "result_schema_version": { "const": "1.0" },
        "firmware_hash": { "type": "string" },
        "config": {
          "required": ["firmware", "system", "script"],
          "properties": {
            "firmware": { "type": "string" },
            "system": { "type": ["string", "null"] },
            "script": { "type": "string" }
          }
        }
      }
    },
    {
      "title": "environment/world result",
      "type": "object",
      "required": ["result_schema_version", "run_type", "config"],
      "not": { "required": ["firmware_hash"] },
      "properties": {
        "result_schema_version": { "const": "1.0-environment" },
        "run_type": { "const": "environment" },
        "config": {
          "required": ["script", "environment", "world_firmware_hash", "nodes"],
          "properties": {
            "script": { "type": "string" },
            "environment": { "type": "string" },
            "world_firmware_hash": { "type": "string" },
            "nodes": {
              "type": "array",
              "items": {
                "required": ["id", "system", "firmware", "system_hash", "firmware_hash"]
              }
            }
          }
        }
      }
    }
  ]
}
```

The environment arm deliberately has no top-level `firmware_hash`; it carries
a whole-world identity in `config.world_firmware_hash` and per-node system and
firmware provenance in `config.nodes`. Environment snapshots use
`type: "environment"` and a `nodes` array rather than a single CPU snapshot.
A rejected `config_overrides` field, non-Arm system/chip or firmware ELF
outside the Cortex-M-only world boundary, or invalid
`uart_cross_link`/`can_bus`/`egress` membership is still emitted as this
environment result arm (with a configuration error) when an output directory
is requested: `status: "error"` and `stop_reason: "config_error"`.

### 4.2 Value Change Dump (`trace.vcd`)

If VCD tracing is enabled, LabWired emits an IEEE 1364-2001 compliant `.vcd` trace capable of being inspected in GTKWave or PulseView.

### 4.3 JUnit XML (`junit.xml`)

Standard CI test reporting format mapped directly from `test_script.yaml` assertions.

---

## 5. Error Taxonomy & Exit Codes

LabWired CI runners exit with specific, predictable status codes.

| Exit Code | Constant Name | Semantics | Protocol Action |
| --- | --- | --- | --- |
| `0` | `EXIT_PASS` | All assertions passed, simulation hit expected terminal condition. | Treat as CI Success. |
| `1` | `EXIT_ASSERT_FAIL` | At least one assertion failed (e.g., missing UART string, unexpected stop reason). | Treat as CI Failure (Logic Error). |
| `2` | `EXIT_CONFIG_ERROR` | Schema validation failed, missing files, or bad YAML. | Fix configuration inputs before retry. |
| `3` | `EXIT_RUNTIME_ERROR`| Internal simulation panic or unrecoverable error. | Report issue / Check hardware compatibility. |

### 5.1 Stop Reasons

The `stop_reason` represents the exact trigger that transitioned the simulator
out of the Execution Loop. Result JSON uses snake-case values:

- `config_error`: Configuration or parsing failed.
- `max_steps`: Exceeded `max_steps`.
- `max_cycles`: Exceeded `max_cycles`.
- `max_uart_bytes`: Exceeded `max_uart_bytes`.
- `no_progress`: The single-machine CPU made no progress for its configured limit.
- `wall_time`: Exceeded `wall_time_ms`.
- `memory_violation`: Accessed unmapped memory or violated access permissions.
- `decode_error`: Encountered an invalid opcode.
- `halt`: Reached a software breakpoint or halted intentionally.
- `exception`: The runner encountered another unrecoverable simulation exception.

---

## 6. Compatibility and Versioning Policy

LabWired's Simulation Protocol follows strict Semantic Versioning.

*   **Schema Versioning**: `schema_version: "1.0"` declarations are frozen. Any breaking changes to field names, allowed values, or structured outputs will require a schema bump to `v2.0` or `v1.1`.
*   **Deprecation**: Legacy formats (e.g., the top-heavy `v1` legacy script) are guaranteed to be supported. A deprecation warning will be printed to `stderr` indicating the migration path.
*   **Forward Compatibility**: Unknown fields inside manifests will generally be rejected to ensure reproducible, strict execution (i.e., `deny_unknown_fields`).

---

## 7. Future-Proofing Extensibility Hooks

The Simulation Protocol is designed to be future-proof against upcoming shifts in hardware simulation requirements. The following schemas and interactions are reserved for future major and minor releases without breaking the core `1.x` execution bounds:

*   **Hybrid Co-Simulation (RTL Integration)**: Future protocol additions will allow binding `peripheral.yaml` registers to Verilator/C++ RTL models via high-speed, zero-copy shared memory IPC interfaces.
*   **Agentic AI API (MAESTRO Framework)**: The simulator will expose a gRPC/WebSocket streaming endpoint synchronized to this protocol's lifecycle (Reset -> Execution -> Halt). This allows external AI agents to read the virtual memory map, evaluate register states, map vulnerabilities, and trigger `AgentIntervention` limits autonomously.
*   **Multi-Physics & FMI 3.0**: Future revisions will support `board_io` mapping extensions. IO pins will map not just to basic UI components (LEDs), but to external FMU (Functional Mock-up Unit) properties modeling dynamic Battery State-of-Charge and CPU Thermal throttling behavior in time-sync with the Execution Loop.
