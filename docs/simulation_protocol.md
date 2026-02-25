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
5.  **Artifact Emission**: The runner flushes final VCD traces, emits `result.json` and JUnit XML, and exits with a standardized code.

---

## 2. Input Manifests (v1.0)

LabWired relies on declarative configuration files to define the "digital twin" of your hardware.

### 2.1 Test Script (`test_script.yaml`)

The test script dictates the bounds and expectations of the simulation.

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
  max_energy_joules: 5.0             # Future (v1.1): Halt if estimated energy exceeds budget
assertions:
  - expected_stop_reason: halt       # Halt instruction (e.g., BKPT, WFI loop)
  - uart_contains: "TEST PASSED"     # Substring match on the UART output stream
```

### 2.2 System Manifest (`system.yaml`)

Defines the board topology, Memory Management overrides, and IO mappings.

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

### 2.3 Environment Manifest (`environment.yaml` - Future v1.1)

To support Distributed Time-Travel Debugging (via Chandy-Lamport algorithms), the protocol introduces an environment manifest grouping multiple systems into a cluster.

```yaml
schema_version: "1.1-draft"
name: "mesh-network-test"
nodes:
  - id: "gateway"
    system: "gateway.yaml"
  - id: "sensor-01"
    system: "sensor.yaml"
interconnects:
  - type: "virtual_switch"
    nodes: ["gateway", "sensor-01"]
```

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

Provides programmatic access to the simulation's final state and metrics.

```json
{
  "result_schema_version": "1.0",
  "status": "passed",
  "steps_executed": 451203,
  "cycles": 620001,
  "instructions": 451203,
  "energy_estimated_joules": 0.045, 
  "carbon_impact_grams": 0.002,     
  "stop_reason": "halt",
  "stop_reason_details": {
    "triggered_stop_condition": "halt",
    "triggered_limit": null,
    "observed": {
      "name": "pc",
      "value": 134218844
    }
  },
  "limits": {
    "max_steps": 100000000
  },
  "assertions": [
    {
      "assertion": { "expected_stop_reason": { "expected_stop_reason": "halt" } },
      "passed": true
    }
  ],
  "firmware_hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "config": {
    "firmware": "test.elf",
    "system": "system.yaml",
    "script": "test.yaml"
  }
}
```

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

The `stop_reason` represents the exact trigger that transitioned the simulator out of the Execution Loop:

- `ConfigError`: Configuration or parsing failed.
- `MaxSteps`: Exceeded `max_steps` limit.
- `MaxCycles`: Exceeded `max_cycles` limit.
- `MaxUartBytes`: Exceeded `max_uart_bytes` limit.
- `MaxEnergy`: (Future) Exceeded `max_energy_joules` sustainability budget.
- `NoProgress`: CPU is spinning without meaningful state change (e.g., stuck in a tight loop reading the same address).
- `WallTime`: Exceeded `wall_time_ms`.
- `MemoryViolation`: Accessing unmapped memory or violating access permissions.
- `DecodeError`: Encountered an invalid opcode.
- `Halt`: The CPU hit a software breakpoint or halted intentionally.
- `AgentIntervention`: (Future) An external AI actor issued a halt command via the MAESTRO API.
- `FmiTimeout`: (Future) Hardware-in-the-loop (FMI 3.0) plant simulation diverged.

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
