# Architecture Internals

A reference for the engine's runtime internals. For the high-level subsystem tour (Asset Foundry, IR, Core Engine), see [architecture_overview.md](architecture_overview.md).

LabWired is a modular execution engine designed to decouple the CPU core from the memory and peripheral bus. This design enables the simulation of multi-architecture systems within a unified peripheral environment.

### Authoritative machine advancement

Ordinary native, CLI-test, debugger, and WASM execution enters through
`Machine::advance`. It owns batch planning, CPU0/CPU1 scheduling, cycle
publication, peripheral ticks, scheduler delivery, software reset, flash
operations, profiling, and logic observation. Frontends inspect the machine
between bounded calls for assertions, stimuli, UART limits, and artifacts, but
must not call `Cpu::step` directly.

The remaining direct CPU advancement sites are deliberately limited to these
implementation, compatibility, and test boundaries:

- `crates/core/src/machine/boundary.rs` is the internal CPU execution primitive
  owned by `Machine::advance`; it is not a frontend entry point.
- `crates/core/src/lib.rs` retains `step_legacy_for_test` and
  `run_legacy_for_test` only as `cfg(test)` differential oracles. CPU trait
  implementations, the default `step_batch` helper, and CPU unit helpers
  likewise operate below or outside the frontend boundary. The direct calls in
  `crates/core/src/peripherals/esp_xtensa_common/rom_thunks.rs` are unit-test
  helpers.
- `crates/core/src/vfi.rs` provides the specialist standalone `ShadowEngine`.
  Its lockstep operation deliberately owns direct stepping of the golden and
  shadow CPUs plus parity comparison; its caller owns the surrounding bus and
  peripheral lifecycle. It is not an ordinary `Machine` frontend.
- `crates/core/src/multi_core.rs` provides the specialist standalone
  `MultiCoreMachine::step_all` engine. It directly steps each core, ticks its
  peripherals once, and routes interrupts according to that engine's own
  policy; it is outside the `Machine` frontend lifecycle.
- `crates/cli/src/main.rs` retains direct stepping only in the temporary,
  test-selected `ExecutionEngine::Legacy` fidelity branch. The default
  `ExecutionEngine::Unified` branch uses `Machine::advance`.
- `crates/cli/src/commands/run.rs` and
  `crates/cli/src/commands/snapshot.rs` contain specialized bare-CPU ESP
  bring-up and snapshot loops; these are explicitly outside the ordinary
  machine frontend path.
- `crates/hw-oracle/src/lib.rs`, `crates/hw-oracle/src/arm_thumb.rs`, and
  `crates/hw-oracle/src/riscv.rs` are bare-CPU hardware validation oracles.

Ordinary WASM wrappers and `DebugControl::run` have no direct CPU-step path;
they enter through the authoritative machine boundary.

## 1. Core Execution Engine (`labwired-core`)

The `labwired-core` crate provides the central execution loop and state management.

### Pluggable CPU Abstraction
The execution engine is generic over a `Cpu` trait, allowing for different instruction set architectures (ISAs) to interface with the same system bus.

```rust
pub trait Cpu {
    /// Resets the CPU state (PC, SP, etc.)
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()>;
    
    /// Executes a single instruction cycle
    fn step(
        &mut self, 
        bus: &mut dyn Bus, 
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig
    ) -> SimResult<()>;
}
```

The system currently implements:
- **Cortex-M (ARMv7-M)**: Supports Thumb-2 instruction decoding.
- **RISC-V (RV32I)**: Supports base integer instruction set.

### Memory Model
The memory system uses a linear addressing model mapped to host memory regions.
- **Flash**: Read-only segments populated from the ELF binary.
- **RAM**: Read-write segments initialized to zero.
- **MMIO**: Addresses outside predefined memory regions are routed to the Peripheral Bus.

## 2. Peripheral Interface

Peripherals communicate with the CPU via the `Peripheral` trait. This trait defines the contract for Memory-Mapped I/O (MMIO) and time-based state updates.

```rust
pub trait Peripheral {
    fn read(&self, offset: u64) -> SimResult<u8>;
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;
    fn tick(&mut self) -> PeripheralTickResult;
}
```

This model prevents race conditions where a peripheral modifies memory while the CPU is executing, ensuring strict sequential consistency.

### Optimized Execution
To achieve high MIPS (Million Instructions Per Second) for autonomous agents, LabWired supports configurable performance gates:
- **Instruction Decode Cache**: A direct-mapped cache in the CPU core that avoids re-decoding instructions on every hit.
- **Multi-Byte Bus Fast-Path**: Specialized 16/32-bit access methods in `SystemBus` that bypass the virtual `read_u8` loop for memory regions (RAM/Flash).
- **Batched Ticking**: Configurable `peripheral_tick_interval`. Ticking every N cycles instead of every instruction significantly reduces virtual call overhead in the hot path.

Defaults and gating are controlled via `SimulationConfig`. Setting `peripheral_tick_interval` to 1 and disabling caches restores strict cycle-accurate behavior for time-sensitive firmware.

## 3. Thumb-2 Decoder

The Cortex-M implementation uses a custom stateless decoder for the ARMv7-M Thumb-2 instruction set.

**Supported Instruction Classes**:
- **32-bit Instructions**: `BL`, `MOVW`, `MOVT` (handled via double half-word fetch).
- **Control Flow**: `B`, `BL`, `BX`, `CBZ/CBNZ`, `IT` blocks.
- **Arithmetic/Logic**: `ADD`, `SUB`, `MUL`, `SDIV/UDIV`, `AND`, `ORR`, `EOR`.
- **Bit Manipulation**: `BFI`, `UBFX`, `CLZ`, `RBIT`.

## 4. Debug Integration

LabWired integrates with external debuggers via standard protocols.

### GDB Remote Serial Protocol (RSP)
The `labwired-gdbstub` crate implements the RSP server, allowing `gdb-multiarch` to attach to the simulation. It supports:
- Breakpoints (Software/Hardware)
- Single-stepping
- Register and Memory inspection

### Debug Adapter Protocol (DAP)
The `labwired-dap` crate provides a direct interface for VS Code. It exposes:
- **State Inspection**: Live view of registers and call stack.
- **Telemetry**: A custom event stream for real-time performance metrics (Cycles, MIPS) without polling overhead.
