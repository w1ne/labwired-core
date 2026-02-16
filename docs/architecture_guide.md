# LabWired Architecture Guide

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
        
        Machine --> CPU[CPU (ARM/RISC-V)]
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
*   **Cpu Trait**: Abstract interface allowing `Cortex-M` or `RISC-V` implementations to be swapped.
*   **SystemBus**: dynamically routes memory accesses (`read`/`write`) to:
    *   **Linear Memory**: RAM/Flash (byte arrays).
    *   **Peripherals**: Structs implementing the `Peripheral` trait.

### Two-Phase Execution (State & Side-Effects)
To satisfy Rust's borrow checker and ensure determinism:
1.  **Tick Phase**: Peripherals update internal state and return *Request Objects* (e.g., `DmaRequest`, `InterruptRequest`).
2.  **Resolution Phase**: The Bus processes these requests, modifying memory or triggering CPU exceptions.

## 3. Peripheral Modeling
We prioritize **Tier 1 Devices** for deep support (see `docs/release_strategy.md`):
*   **STM32F4** (Cortex-M4)
*   **RP2040** (Dual Cortex-M0+)
*   **nRF52** (Cortex-M4F)

Peripherals are implemented as Rust structs that mimic hardware logic (registers, bitfields, state machines).

## 4. Interfaces (`crates/cli`, `crates/dap`)
*   **CLI**: The main entry point. Runs simulations, imports assets, and manages configuration.
*   **DAP Server**: Implements the Debug Adapter Protocol for seamless VS Code integration.
    *   Typically listens on TCP `5000`.
    *   Provides specialized telemetry events (PC, Cycles, Power) to the IDE.

## Directory Structure
*   `core/crates/ir`: Strict IR definitions and SVD transformation logic.
*   `core/crates/core`: CPU, Bus, and Device traits.
*   `core/crates/cli`: Command-line driver.
*   `core/crates/loader`: ELF parsing.
*   `core/crates/config`: Configuration file parsing (system manifest).
