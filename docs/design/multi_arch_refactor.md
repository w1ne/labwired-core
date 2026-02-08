# Multi-Architecture Support Refactoring

## Context
LabWired core is currently tightly coupled to ARM Cortex-M architecture, specifically regarding system peripherals like NVIC and SCB, and the Vector Table Offset Register (VTOR). To support other architectures (RISC-V, AVR, etc.), we need to decouple the generic `Machine` and `Cpu` trait from these specifics.

## Goals
- Remove hardcoded dependencies on ARM Cortex-M architecture (NVIC, SCB, VTOR) from the generic `Cpu` trait.
- Make `Machine` struct a pure execution container, agnostic of specific system peripherals.
- Introduce specific system configuration helpers for Cortex-M.

## Implementation Plan

### 1. Refactor `Cpu` Trait
**File:** `crates/core/src/cpu/mod.rs`

- Remove `get_vtor`, `set_vtor`, `set_shared_vtor` from `Cpu` trait.
- Keep `set_exception_pending` as it is a generic concept.
- `CortexM` implementation will retain `vtor` logic but it will not be exposed via the generic trait.

### 2. Decouple `Machine` from System Peripherals
**File:** `crates/core/src/lib.rs`

- Remove `Machine::with_bus()` logic that automatically instantiates and attaches `Nvic` and `Scb` peripherals.
- `Machine` should only accept an already configured `Bus` and `Cpu`.

### 3. Introduce System Helper
**File:** `crates/core/src/lib.rs` (or a sidebar module)

- Create a helper function/struct `configure_cortex_m(bus: &mut SystemBus) -> (CortexM, Arc<NvicState>)` or similar.
- This helper will:
    - Create shared `AtomicU32` for VTOR.
    - Create `CortexM` CPU instance with this VTOR.
    - Create `Nvic` and `Scb` peripherals, sharing the VTOR and NVIC state.
    - Attach peripherals to the bus.

### 4. Update Consumers (CLI & Tests)
**File:** `crates/cli/src/main.rs`, `crates/core/tests.rs`

- Update `run_interactive` and `run_test` to explicitly call the Cortex-M configuration logic before creating the `Machine`.

## Status: Completed
The multi-architecture refactor has been successfully implemented:
-   **Core Interface**: Generic `Cpu` trait is decoupled from ARM-specific registers.
-   **Machine Initialization**: `Machine` is now architecture-agnostic, with system setup helpers for Cortex-M and RISC-V.
-   **Snapshots**: `CpuSnapshot` refactored into an enum, supporting standardized state capture for both ARM and RISC-V.
-   **Test Runner**: The `labwired test` command automatically detects the architecture from ELF files and runs the appropriate simulation loop.
-   **Regression Testing**: CI pipeline validated with real ELF binaries for ARM (M0, M3) and RISC-V.
