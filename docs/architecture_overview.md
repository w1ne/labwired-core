# LabWired Architecture Overview

A high-level tour of the simulator's subsystems and how they fit together. For engine internals (CPU trait, decoder, performance gates, debug protocols), see [architecture.md](architecture.md).

**LabWired** is a modular, high-fidelity embedded systems simulator written in Rust. It isolates hardware description (Asset Foundry) from execution logic (Core Engine), linked by a Strict Intermediate Representation (IR).

```mermaid
graph TD
    %% Subsystems
    subgraph "Asset Foundry (Ingestion)"
        SVD[Vendor SVD] -->|import-svd| IR[Strict IR (JSON)]
        IR -->|Codegen| RustModel[Rust Core Model]
    end

    subgraph "Execution Engine (Core)"
        ELF[Firmware ELF] --> Loader
        Loader --> Machine
        RustModel --> Machine
        
        Machine --> CPU[CPU (ARM/RISC-V/Xtensa)]
        Machine --> Bus[System Bus]
        Bus --> Memory
        Bus --> Peripherals
    end

    subgraph "Interfaces"
        CLI[labwired-cli] --> Machine
        GDB[GDB Stub] -.-> Machine
        DAP[VS Code DAP] -.-> Machine
    end
```

## 1. Asset Foundry (`crates/ir`)
The **Asset Foundry** is the supply chain for simulation models. It solves the problem of "dirty" vendor data.

*   **Input**: Vendor SVD files (often broken, inconsistent).
*   **Process**: Use `labwired asset import-svd` to flatten, unroll, and sanitize.
*   **Output**: **Strict IR** (`labwired-ir`). A JSON format where:
    *   All arrays (`UART0`, `UART1`) are unrolled.
    *   All inheritance (`derivedFrom`) is resolved.
    *   All clusters are flattened.
*   **Goal**: Zero-ambiguity input for the simulation core.

## 2. The Core Engine (`crates/core`)
The simulation runtime. It is `no_std` compatible and designed for deterministic execution.

### Components
*   **Machine**: The top-level container holding CPU, Bus, and Peripherals.
*   **Cpu Trait**: Abstract interface allowing `Cortex-M`, `RISC-V`, or `Xtensa` implementations to be swapped.
*   **SystemBus**: dynamically routes memory accesses (`read`/`write`) to:
    *   **Linear Memory**: RAM/Flash (byte arrays).
    *   **Peripherals**: Structs implementing the `Peripheral` trait.

### Two-Phase Execution (State & Side-Effects)
To satisfy Rust's borrow checker and ensure determinism:
1.  **Tick Phase**: Peripherals update internal state and return *Request Objects* (e.g., `DmaRequest`, `InterruptRequest`).
2.  **Resolution Phase**: The Bus processes these requests, modifying memory or triggering CPU exceptions.

## 3. Peripheral Modeling
Coverage is intentionally uneven — depth is driven per chip rather than uniformly.
The chip family spans Cortex-M (STM32F1/F4/G4/H5/L0/L4/WB, nRF52/53), RISC-V
(ESP32-C3), and Xtensa (ESP32-S3); see
[`docs/coverage_scoreboard.md`](coverage_scoreboard.md) for the live per-chip
conformance and validation matrices.

Peripherals are implemented as Rust structs that mimic hardware logic (registers, bitfields, state machines).

## 4. Interfaces (`crates/cli`, `crates/dap`)
*   **CLI**: The main entry point. Runs simulations, imports assets, and manages configuration.
*   **DAP Server**: Implements the Debug Adapter Protocol for seamless VS Code integration.
    *   Typically listens on TCP `5000`.
    *   Provides specialized telemetry events (PC, Cycles, Power) to the IDE.

## Directory Structure
*   `crates/ir`: Strict IR definitions and SVD transformation logic.
*   `crates/core`: CPU, Bus, and Device traits.
*   `crates/cli`: Command-line driver.
*   `crates/loader`: ELF parsing.
*   `crates/config`: Configuration file parsing (system manifest).
