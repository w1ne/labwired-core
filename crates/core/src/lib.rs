#![allow(clippy::manual_is_multiple_of)]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod boot;
pub mod bus;
pub mod config;
pub mod cosim;
pub mod cpu;
pub mod decoder;
pub mod interrupt;
pub mod memory;
pub mod metrics;
pub mod multi_core;
pub mod network;
pub mod peripherals;
pub mod physics;
pub mod runtime_snapshot;
pub mod signals;
pub mod snapshot;
pub mod system;
pub mod trace;
pub mod vfi;
pub mod world;

pub use config::SimulationConfig;

use std::any::Any;
use std::sync::Arc;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Arch {
    Arm,
    RiscV,
    XtensaLx7,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum SimulationError {
    #[error("Memory access violation at {0:#x}")]
    MemoryViolation(u64),
    #[error("Instruction decoding error at {0:#x}")]
    DecodeError(u64),
    #[error("Simulation halted")]
    Halt,
    #[error("Snapshot schema mismatch: expected v{expected}, got v{got}")]
    SnapshotSchemaMismatch { expected: u32, got: u32 },
    #[error("not implemented: {0}")]
    NotImplemented(String),
    #[error("Breakpoint hit at {0:#x}")]
    BreakpointHit(u32),
    #[error("Exception raised: cause={cause} at pc={pc:#x}")]
    ExceptionRaised { cause: u8, pc: u32 },
    #[error("Simulation error: {0}")]
    Other(String),
}

pub type SimResult<T> = Result<T, SimulationError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    Read,
    Write,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaRequest {
    pub src_addr: u64,
    pub addr: u64,
    pub val: u8,
    pub direction: DmaDirection,
}

#[derive(Debug, Clone, Default)]
pub struct PeripheralTickResult {
    /// Raises the peripheral's configured NVIC IRQ position (set in
    /// `PeripheralEntry::irq`). The bus pends it on NVIC.ISPR so the
    /// CPU only takes it when ISER also has it enabled.
    pub irq: bool,
    pub cycles: u32,
    pub dma_requests: Option<Vec<DmaRequest>>,
    /// Extra NVIC IRQ positions beyond the peripheral's default.
    pub explicit_irqs: Option<Vec<u32>>,
    /// System exception number (0..15) to raise — for SysTick (15) and
    /// other core-tied exceptions that bypass NVIC. Pushed directly to
    /// the CPU's pending_exceptions bitmap; PRIMASK still gates it.
    pub system_exception: Option<u32>,
    pub dma_signals: Option<Vec<u32>>,
    pub ticks_until_next: Option<u64>,

    /// Cross-peripheral side-effect writes the peripheral wants the bus
    /// to apply on its behalf. Used by GPIOTE to drive GPIO OUTSET/OUTCLR
    /// without holding a bus handle, and as the sink for PPI-triggered
    /// task writes.
    pub mmio_writes: Vec<(u32, u32)>,

    /// Offsets (within this peripheral's window) of EVENTS_* registers
    /// that transitioned 0→1 during this tick. The bus globalises them
    /// to absolute addresses (peripheral.base + offset) and feeds them
    /// to the PPI router. Peripherals that don't fire events leave this
    /// empty; consumers ignore it.
    pub fired_events: Vec<u32>,
}

impl PeripheralTickResult {
    pub fn with_irq(irq: bool) -> Self {
        Self {
            irq,
            ..Default::default()
        }
    }
}

/// Trait for observing simulation events in a modular way.
pub trait SimulationObserver: std::fmt::Debug + Send + Sync {
    fn on_simulation_start(&self) {}
    fn on_simulation_stop(&self) {}
    fn on_step_start(&self, _pc: u32, _opcode: u32) {}
    fn on_step_end(&self, _cycles: u32, _registers: &[u32]) {}
    fn on_memory_write(&self, _addr: u64, _old: u8, _new: u8) {}
    fn on_peripheral_tick(&self, _name: &str, _cycles: u32) {}
}

/// Trait representing a CPU architecture
pub trait Cpu: Send {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()>;
    /// Downcast escape hatch for runtime fast-paths that need the
    /// concrete CPU type (e.g. the browser-side JIT prototype in
    /// `labwired-wasm` reaches into `XtensaLx7` for direct register
    /// access). Default returns `None`, matching the rest of the trait
    /// surface: only the CPUs that opt in are reachable this way.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()>;
    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        for _ in 0..max_count {
            self.step(bus, observers, config)?;
        }
        Ok(max_count)
    }
    fn set_pc(&mut self, val: u32);
    fn get_pc(&self) -> u32;
    fn set_sp(&mut self, val: u32);
    fn set_exception_pending(&mut self, exception_num: u32);

    // Debug Access
    fn get_register(&self, id: u8) -> u32;
    fn set_register(&mut self, id: u8, val: u32);
    fn snapshot(&self) -> snapshot::CpuSnapshot;
    fn apply_snapshot(&mut self, snapshot: &snapshot::CpuSnapshot);

    /// Full mid-flight CPU state for binary runtime snapshots. Returns the
    /// arch tag + an opaque blob the matching CPU type knows how to parse.
    /// Default panics — every concrete `Cpu` impl must override.
    fn runtime_snapshot(&self) -> (runtime_snapshot::CpuKind, Vec<u8>) {
        unimplemented!("runtime_snapshot not implemented for this Cpu")
    }

    /// Apply a previously-taken runtime snapshot. Default no-op so
    /// stub/test CPUs don't need to override.
    fn apply_runtime_snapshot(
        &mut self,
        _kind: runtime_snapshot::CpuKind,
        _bytes: &[u8],
    ) -> SimResult<()> {
        Ok(())
    }
    fn get_register_names(&self) -> Vec<String>;
    fn index_of_register(&self, name: &str) -> Option<u8>;

    // Security & Physical Extensions
    fn inject_fault(&mut self, _target: &str) -> SimResult<()> {
        Ok(())
    }
    fn get_energy_consumption(&self) -> f64 {
        0.0
    }

    /// Set bits in the pending-interrupt register. Default no-op; Xtensa
    /// implementations latch the bits into INTERRUPT (SR id 226) so a
    /// cross-core IPI bridge in the host runtime can synthesise edges
    /// without owning the SR file directly.
    fn raise_interrupt_bits(&mut self, _mask: u32) {}

    /// Halt this CPU: subsequent `step()` calls are no-ops until
    /// `unhalt()`. Models reset-hold for dual-core SoCs where one core
    /// is held in reset until the bootstrap CPU releases it. Default
    /// no-op so single-core CPUs need no implementation.
    fn halt(&mut self) {}
    /// Release a previously-halted CPU; pairs with [`Self::halt`].
    fn unhalt(&mut self) {}

    /// Current interrupt-mask level. Used by dual-core schedulers to
    /// serialize critical sections — when one CPU has intlevel > 0
    /// (typically because portENTER_CRITICAL raised it to 3 to hold a
    /// spinlock), the sim runs that CPU solo until it drops back to 0.
    /// Default returns 0 so single-core CPUs need no implementation.
    fn intlevel(&self) -> u8 {
        0
    }

    /// Phase 3.2 JIT pilot (issue #124): total number of times any
    /// JIT-compiled block on this CPU has been invoked. Default 0 for
    /// CPUs/builds without JIT support so callers can unconditionally
    /// query it in reporting paths.
    fn jit_hit_count(&self) -> u64 {
        0
    }
}

// Forwarding impl so `Machine<Box<dyn Cpu>>` is valid — used by the WASM
// runtime to hold an arch-dispatched CPU without making WasmSimulator
// generic over C: Cpu (wasm-bindgen can't expose a generic struct).
impl Cpu for Box<dyn Cpu> {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()> {
        (**self).reset(bus)
    }
    /// Forward the concrete-type escape hatch through the Box.
    /// Without this, `Machine<Box<dyn Cpu>>::cpu.as_any_mut()` would
    /// hit the default impl on the trait (which returns `None`),
    /// silently disabling every downcast — including the browser JIT
    /// dispatcher in `labwired-wasm`. Reported during #124 Phase 4.2
    /// bench validation.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        (**self).as_any_mut()
    }
    fn step(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<()> {
        (**self).step(bus, observers, config)
    }
    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        (**self).step_batch(bus, observers, config, max_count)
    }
    fn set_pc(&mut self, val: u32) {
        (**self).set_pc(val)
    }
    fn get_pc(&self) -> u32 {
        (**self).get_pc()
    }
    fn set_sp(&mut self, val: u32) {
        (**self).set_sp(val)
    }
    fn set_exception_pending(&mut self, n: u32) {
        (**self).set_exception_pending(n)
    }
    fn get_register(&self, id: u8) -> u32 {
        (**self).get_register(id)
    }
    fn set_register(&mut self, id: u8, val: u32) {
        (**self).set_register(id, val)
    }
    fn snapshot(&self) -> snapshot::CpuSnapshot {
        (**self).snapshot()
    }
    fn apply_snapshot(&mut self, s: &snapshot::CpuSnapshot) {
        (**self).apply_snapshot(s)
    }
    fn runtime_snapshot(&self) -> (runtime_snapshot::CpuKind, Vec<u8>) {
        (**self).runtime_snapshot()
    }
    fn apply_runtime_snapshot(
        &mut self,
        kind: runtime_snapshot::CpuKind,
        bytes: &[u8],
    ) -> SimResult<()> {
        (**self).apply_runtime_snapshot(kind, bytes)
    }
    fn get_register_names(&self) -> Vec<String> {
        (**self).get_register_names()
    }
    fn index_of_register(&self, name: &str) -> Option<u8> {
        (**self).index_of_register(name)
    }
    fn inject_fault(&mut self, target: &str) -> SimResult<()> {
        (**self).inject_fault(target)
    }
    fn get_energy_consumption(&self) -> f64 {
        (**self).get_energy_consumption()
    }
    fn raise_interrupt_bits(&mut self, mask: u32) {
        (**self).raise_interrupt_bits(mask)
    }
    fn halt(&mut self) {
        (**self).halt()
    }
    fn unhalt(&mut self) {
        (**self).unhalt()
    }
    fn intlevel(&self) -> u8 {
        (**self).intlevel()
    }
    fn jit_hit_count(&self) -> u64 {
        (**self).jit_hit_count()
    }
}

/// Trait representing a memory-mapped peripheral
pub trait Peripheral: std::fmt::Debug + Send {
    fn read(&self, offset: u64) -> SimResult<u8>;
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>;
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let b0 = self.read(offset)? as u16;
        let b1 = self.read(offset + 1)? as u16;
        Ok(b0 | (b1 << 8))
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let b0 = self.read(offset)? as u32;
        let b1 = self.read(offset + 1)? as u32;
        let b2 = self.read(offset + 2)? as u32;
        let b3 = self.read(offset + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write(offset + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write(offset + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }
    /// Plan 2: word-granular write path. The bus calls this after performing
    /// the four byte writes, giving peripherals a single coherent 32-bit
    /// view of the write. Default: no-op. Peripherals with 32-bit word
    /// triggers (e.g. declarative configs with WriteWord triggers) override.
    fn write_word_32(&mut self, _offset: u64, _value: u32) -> SimResult<()> {
        Ok(())
    }
    /// Side-effect-free value probe used for debug/observer bookkeeping.
    /// Implementations should return `None` when such probing is not supported.
    fn peek(&self, _offset: u64) -> Option<u8> {
        None
    }
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
    /// PPI hook: given absolute addresses of events that just fired
    /// across the bus, return absolute addresses of tasks to trigger.
    /// Only the PPI peripheral overrides this; everyone else returns
    /// the default empty vector and the bus skips them at near-zero
    /// cost.
    fn route_ppi_events(&mut self, _fired_global: &[u32]) -> Vec<u32> {
        Vec::new()
    }

    /// Cross-peripheral GPIO change hook: bus snapshots GPIO IN registers
    /// each tick and calls this with a list of `(port, pin, new_level)`
    /// transitions. GPIOTE overrides to drive EVENTS_IN[i] when a channel
    /// is configured to watch a matching (port, pin) with a matching
    /// polarity. Default no-op.
    fn observe_gpio_change(&mut self, _changes: &[(u8, u8, u8)]) {}
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
    fn dma_request(&mut self, _request_id: u32) {}
    fn snapshot(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn restore(&mut self, _state: serde_json::Value) -> SimResult<()> {
        Ok(())
    }

    /// Binary mid-flight runtime snapshot — captures whatever state this
    /// peripheral needs for resume. Default is an empty blob: stateless
    /// peripherals can ignore it. Override for RAM regions, framebuffer-
    /// holding panels, sparse register banks, etc.
    ///
    /// Distinct from [`Self::snapshot`], which produces a JSON value for
    /// the determinism gates and human-inspectable replay tools — those
    /// can't carry the megabyte of RAM contents we need for instant
    /// resume.
    fn runtime_snapshot(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Restore from a previously-taken `runtime_snapshot()`. Default is
    /// a no-op: peripherals that don't override `runtime_snapshot`
    /// don't need to override this either.
    fn restore_runtime_snapshot(&mut self, _bytes: &[u8]) -> SimResult<()> {
        Ok(())
    }
}

/// Trait representing the system bus
pub trait Bus {
    fn read_u8(&self, addr: u64) -> SimResult<u8>;
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()>;
    fn tick_peripherals(&mut self) -> Vec<u32>; // Returns list of pending exception numbers
    fn execute_dma(&mut self, requests: &[DmaRequest]) -> SimResult<()>;
    fn config(&self) -> &SimulationConfig;
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }

    /// Clear a pending NVIC exception (called by CPU when taking an exception).
    /// For NVIC exceptions (number >= 16), this clears the ISPR bit to prevent
    /// immediate re-entry of the same exception.
    fn clear_nvic_pending(&mut self, _exception_num: u32) {}

    /// Plan 3: route a peripheral source ID to a cpu0 IRQ slot via a
    /// registered ESP32-S3 interrupt matrix peripheral. Default returns
    /// None for buses that don't model this.
    fn route_irq_source_to_cpu_irq(&self, _source_id: u32) -> Option<u8> {
        None
    }

    /// Plan 3: bitmask of pending cpu0 IRQ slots aggregated by the bus
    /// from peripheral tick results routed through the intmatrix. Default
    /// returns 0; non-ESP32-S3 buses don't model this.
    fn pending_cpu_irqs(&self) -> u32 {
        0
    }

    /// Plan 3: clear the pending bit for cpu IRQ slot `slot`.
    fn clear_cpu_irq_pending(&mut self, _slot: u8) {}

    /// Plan 2: deliver a coherent 32-bit value to peripherals after the
    /// four byte writes that compose a write_u32 have been dispatched.
    /// Default: no-op for buses that don't route to peripherals.
    fn notify_word_write(&mut self, _addr: u64, _value: u32) -> SimResult<()> {
        Ok(())
    }

    /// Plan 3: look up a registered ROM thunk by absolute PC. Used by the
    /// Xtensa LX7 `BREAK 1, 14` dispatch to redirect calls into the simulated
    /// ESP32-S3 mask ROM. Default returns None for buses that don't model
    /// ROM thunks.
    fn get_rom_thunk(
        &self,
        _pc: u32,
    ) -> Option<crate::peripherals::esp32s3::rom_thunks::RomThunkFn> {
        None
    }

    fn read_u16(&self, addr: u64) -> SimResult<u16> {
        let b0 = self.read_u8(addr)? as u16;
        let b1 = self.read_u8(addr + 1)? as u16;
        // Little Endian
        Ok(b0 | (b1 << 8))
    }

    fn read_u32(&self, addr: u64) -> SimResult<u32> {
        let b0 = self.read_u8(addr)? as u32;
        let b1 = self.read_u8(addr + 1)? as u32;
        let b2 = self.read_u8(addr + 2)? as u32;
        let b3 = self.read_u8(addr + 3)? as u32;
        Ok(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
    }

    fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        self.write_u8(addr + 2, ((value >> 16) & 0xFF) as u8)?;
        self.write_u8(addr + 3, ((value >> 24) & 0xFF) as u8)?;
        Ok(())
    }

    fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        self.write_u8(addr, (value & 0xFF) as u8)?;
        self.write_u8(addr + 1, ((value >> 8) & 0xFF) as u8)?;
        Ok(())
    }

    /// Optional fast-path for instruction fetch: return a contiguous
    /// `&[u8]` covering `pc`, plus the absolute `[range_start, range_end)`
    /// it serves. The CPU caches this slice on the side and reads
    /// instructions directly out of it without round-tripping the bus
    /// dispatcher, peripheral lookup, or `RefCell` borrow.
    ///
    /// Default returns `None`; buses that can serve linear memory
    /// (e.g. `SystemBus` when `pc` lands in a `RamPeripheral`) should
    /// return `Some`. Callers MUST treat the slice as read-only and
    /// invalidate their cached pointer on any bus write that may touch
    /// the same range and on snapshot restore. See labwired-core#119
    /// (JIT roadmap Phase 1.2).
    fn fetch_slice(&self, _pc: u64) -> Option<(u64, u64, &[u8])> {
        None
    }
}

use std::collections::HashSet;

/// Trait for controlling the machine in debug mode
pub trait DebugControl {
    fn add_breakpoint(&mut self, addr: u32);
    fn remove_breakpoint(&mut self, addr: u32);
    fn clear_breakpoints(&mut self);

    /// Run until breakpoint or steps limit
    fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason>;

    /// Step a single instruction
    fn step_single(&mut self) -> SimResult<StopReason>;

    fn read_core_reg(&self, id: u8) -> u32;
    fn write_core_reg(&mut self, id: u8, val: u32);

    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>>;
    fn write_memory(&mut self, addr: u32, data: &[u8]) -> SimResult<()>;

    fn get_pc(&self) -> u32;
    fn set_pc(&mut self, addr: u32);
    fn get_register_names(&self) -> Vec<String>;
    fn get_cycle_count(&self) -> u64;
    fn get_peripherals(&self) -> Vec<(String, u64, u64)>;
    fn get_peripheral_descriptor(
        &self,
        name: &str,
    ) -> Option<labwired_config::PeripheralDescriptor>;
    fn reset(&mut self) -> SimResult<()>;

    // State Management
    fn snapshot(&self) -> snapshot::MachineSnapshot;
    fn restore(&mut self, snapshot: &snapshot::MachineSnapshot) -> SimResult<()>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    Breakpoint(u32),
    StepDone,
    MaxStepsReached,
    ManualStop,
}

pub struct Machine<C: Cpu> {
    pub cpu: C,
    /// Secondary CPU instance — for dual-core SoCs (ESP32, ESP32-S3).
    /// When `Some`, `step()` interleaves CPU 0 and CPU 1 instructions
    /// over a shared bus. `None` for everything else (Cortex-M, RP2040,
    /// pre-existing single-core Xtensa configs that don't opt in).
    /// Snapshot/restore APIs currently track only the primary CPU —
    /// callers that need full dual-core snapshots must extend the format.
    pub cpu_secondary: Option<C>,
    pub bus: bus::SystemBus,
    pub observers: Vec<Arc<dyn SimulationObserver>>,

    // Debug state
    pub breakpoints: std::collections::HashSet<u32>,
    pub last_breakpoint: Option<u32>,
    pub total_cycles: u64,
    pub config: SimulationConfig,
}

impl<C: Cpu> Machine<C> {
    pub fn new(cpu: C, bus: bus::SystemBus) -> Self {
        Self {
            cpu,
            cpu_secondary: None,
            bus,
            observers: Vec::new(),
            breakpoints: HashSet::new(),
            last_breakpoint: None,
            total_cycles: 0,
            config: SimulationConfig::default(),
        }
    }

    /// Enable dual-core mode by attaching a secondary CPU instance.
    /// The secondary CPU shares the same bus and steps in lockstep with
    /// the primary (one instruction each per `Machine::step()`). Used by
    /// ESP32 configs to model APP_CPU alongside PRO_CPU.
    pub fn with_secondary_cpu(mut self, cpu1: C) -> Self {
        self.cpu_secondary = Some(cpu1);
        self
    }

    /// Take a binary mid-flight runtime snapshot of the entire machine —
    /// CPU + every peripheral that overrides `runtime_snapshot()`. The
    /// returned blob feeds straight into [`Self::apply_runtime_snapshot`]
    /// on a freshly-constructed `Machine` with the same firmware loaded
    /// and the same bus topology.
    ///
    /// Distinct from [`Self::snapshot`] (which produces a JSON value for
    /// the determinism gates) — this one is binary, captures everything
    /// needed for resume (full SR file, shadow stacks, RAM regions, etc.)
    /// and is what the playground uses to ship pre-warmed boot snapshots
    /// alongside firmware ELFs.
    pub fn take_runtime_snapshot(&self) -> runtime_snapshot::MachineRuntimeSnapshot {
        let (cpu_kind, cpu_data) = self.cpu.runtime_snapshot();
        let peripherals: Vec<(String, Vec<u8>)> = self
            .bus
            .peripherals
            .iter()
            .filter(|entry| {
                !runtime_snapshot::RUNTIME_SNAPSHOT_SKIPPED_PERIPHERALS
                    .iter()
                    .any(|skip| *skip == entry.name)
            })
            .map(|entry| (entry.name.clone(), entry.dev.runtime_snapshot()))
            .collect();
        runtime_snapshot::MachineRuntimeSnapshot::new(cpu_kind, cpu_data, peripherals)
    }

    /// Restore from a previously-taken runtime snapshot. Bus topology
    /// must match (same peripherals registered under the same names) —
    /// peripherals not present in the snapshot keep their current state,
    /// peripherals named in the snapshot but missing on the bus return
    /// `MissingPeripheral` so the caller can fail loudly instead of
    /// silently dropping state.
    pub fn apply_runtime_snapshot(
        &mut self,
        snap: &runtime_snapshot::MachineRuntimeSnapshot,
    ) -> SimResult<()> {
        self.cpu
            .apply_runtime_snapshot(snap.cpu_kind, &snap.cpu_data)?;
        for (name, blob) in &snap.peripherals {
            let entry = self
                .bus
                .peripherals
                .iter_mut()
                .find(|e| &e.name == name)
                .ok_or_else(|| {
                    SimulationError::NotImplemented(format!(
                        "apply_runtime_snapshot: peripheral '{name}' not on bus"
                    ))
                })?;
            entry.dev.restore_runtime_snapshot(blob)?;
        }
        Ok(())
    }
}

impl<C: Cpu> Machine<C> {
    pub fn load_firmware(&mut self, image: &memory::ProgramImage) -> SimResult<()> {
        for segment in &image.segments {
            // 1. Try Cortex-M / ARM flash backing.
            if self.bus.flash.load_from_segment(segment) {
                continue;
            }
            // 2. Try Cortex-M / ARM ram backing.
            if self.bus.ram.load_from_segment(segment) {
                continue;
            }
            // 3. Fall back to peripheral-backed memory (ESP32 IRAM /
            //    flash XIP, RP2040 SRAM, etc — anything registered as a
            //    Peripheral rather than living in bus.flash/bus.ram).
            //    Walk the segment byte-by-byte through the bus dispatcher.
            //    Slow path, but only runs once at load time.
            let mut all_written = true;
            for (i, byte) in segment.data.iter().enumerate() {
                let addr = segment.start_addr + i as u64;
                if self.bus.write_u8(addr, *byte).is_err() {
                    all_written = false;
                    break;
                }
            }
            if !all_written {
                tracing::warn!(
                    "Failed to load segment at {:#x} - outside of memory map",
                    segment.start_addr
                );
            }
        }

        for observer in &self.observers {
            observer.on_simulation_start();
        }
        self.reset()?;

        // Fallback if vector table is missing/zero
        if self.cpu.get_pc() == 0 {
            self.cpu.set_pc(image.entry_point as u32);
        }

        Ok(())
    }

    pub fn reset(&mut self) -> SimResult<()> {
        self.cpu.reset(&mut self.bus)?;
        if let Some(cpu1) = self.cpu_secondary.as_mut() {
            cpu1.reset(&mut self.bus)?;
        }
        Ok(())
    }

    pub fn step(&mut self) -> SimResult<()> {
        self.total_cycles += 1;
        self.cpu
            .step(&mut self.bus, &self.observers, &self.config)?;
        // Dual-core: step the secondary CPU one instruction per
        // primary-CPU instruction (round-robin). Cycle counter only
        // advances for the primary CPU — keeps observer/snapshot
        // semantics stable. Errors on CPU 1 bubble up the same way.
        if let Some(cpu1) = self.cpu_secondary.as_mut() {
            // CPU 0 may have just called `ets_set_appcpu_boot_addr` to
            // release APP_CPU from reset-hold. The thunk stashed the
            // boot address in a thread-local; drain it here, apply to
            // the secondary CPU's PC, and unhalt so the next round-robin
            // tick starts executing from that address.
            if let Some(boot_addr) =
                crate::peripherals::esp32s3::rom_thunks::APPCPU_BOOT_ADDR.with(|s| s.take())
            {
                cpu1.set_pc(boot_addr);
                cpu1.unhalt();
            }
            cpu1.step(&mut self.bus, &self.observers, &self.config)?;
        }

        if self.total_cycles % (self.config.peripheral_tick_interval as u64) == 0 {
            // Propagate peripherals
            let (interrupts, costs) = self.bus.tick_peripherals_fully();
            for c in costs {
                self.total_cycles += c.cycles as u64;
                if let Some(p) = self.bus.peripherals.get(c.index) {
                    for observer in &self.observers {
                        observer.on_peripheral_tick(&p.name, c.cycles);
                    }
                }
            }
            for irq in interrupts {
                self.cpu.set_exception_pending(irq);
                tracing::debug!("Exception {} Pend", irq);
            }
        }

        // RTC_CNTL software system reset (OPTIONS0 bit 31 / `SW_SYS_RST`).
        // The ESP32 BROM's `_rtc_trigger_sw_system_reset` writes this bit
        // and expects execution NOT to return from the store — on real
        // silicon the CPU restarts at the reset vector. We drain the
        // request between instructions so neither the CPU nor any
        // peripheral observes a half-applied state. Reset vector for the
        // ESP32 rev3 BROM `_ResetVector` is fixed at `0x4000_0400`; SP is
        // re-seeded to the top of DRAM the BROM uses (`0x3FFE_0000`),
        // matching the smoke-test cold-boot setup.
        if self.drain_rtc_cntl_reset_request() {
            self.cpu.set_pc(0x4000_0400);
            self.cpu.set_sp(0x3FFE_0000);
            tracing::debug!("RTC_CNTL SW_SYS_RST: CPU re-pointed at reset vector 0x40000400");
        }

        Ok(())
    }

    /// Returns true (and clears the latch) if any registered RTC_CNTL
    /// peripheral has a pending software-system-reset request. Used by
    /// `step()` to honor OPTIONS0 bit 31 writes at a clean instruction
    /// boundary. Walks the bus's peripheral list and downcasts; in
    /// practice there's at most one RTC_CNTL on the bus, so this is O(N)
    /// over a short vector.
    fn drain_rtc_cntl_reset_request(&self) -> bool {
        for p in &self.bus.peripherals {
            if let Some(any) = p.dev.as_any() {
                if let Some(rtc) =
                    any.downcast_ref::<crate::peripherals::esp32::rtc_cntl::RtcCntl>()
                {
                    if rtc.drain_reset_request() {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn snapshot(&self) -> snapshot::MachineSnapshot {
        snapshot::MachineSnapshot {
            schema_version: snapshot::SCHEMA_VERSION,
            cpu: self.cpu.snapshot(),
            peripherals: self
                .bus
                .peripherals
                .iter()
                .map(|p| (p.name.clone(), p.dev.snapshot()))
                .collect(),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: snapshot::MachineSnapshot) -> SimResult<()> {
        if snapshot.schema_version != snapshot::SCHEMA_VERSION {
            return Err(SimulationError::SnapshotSchemaMismatch {
                expected: snapshot::SCHEMA_VERSION,
                got: snapshot.schema_version,
            });
        }
        self.cpu.apply_snapshot(&snapshot.cpu);
        for p in &mut self.bus.peripherals {
            if let Some(state) = snapshot.peripherals.get(&p.name) {
                p.dev.restore(state.clone())?;
            }
        }
        Ok(())
    }

    pub fn peek_peripheral(&self, name: &str) -> Option<serde_json::Value> {
        self.bus
            .peripherals
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.dev.snapshot())
    }
}

impl<C: Cpu> DebugControl for Machine<C> {
    fn add_breakpoint(&mut self, addr: u32) {
        self.breakpoints.insert(addr);
    }

    fn remove_breakpoint(&mut self, addr: u32) {
        self.breakpoints.remove(&addr);
    }

    fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason> {
        let mut steps = 0;

        loop {
            // Check breakpoints BEFORE batch
            let pc = self.cpu.get_pc();
            let pc_aligned = pc & !1;

            if self.breakpoints.contains(&pc_aligned) && self.last_breakpoint != Some(pc_aligned) {
                self.last_breakpoint = Some(pc_aligned);
                return Ok(StopReason::Breakpoint(pc));
            }

            // We are executing, so clear the "last hit" sticky BP
            self.last_breakpoint = None;

            if let Some(limit) = max_steps {
                if steps >= limit {
                    return Ok(StopReason::MaxStepsReached);
                }
            }

            // Execute in batch until next peripheral tick or breakpoint/limit
            let current_cycles = self.total_cycles;
            let tick_interval = self.config.peripheral_tick_interval as u64;
            let remaining_until_tick = (tick_interval - (current_cycles % tick_interval)) as u32;

            let current_batch = if let Some(limit) = max_steps {
                remaining_until_tick.min(limit - steps)
            } else {
                remaining_until_tick
            };

            let executed =
                self.cpu
                    .step_batch(&mut self.bus, &self.observers, &self.config, current_batch)?;

            steps += executed;
            self.total_cycles += executed as u64;

            if self.total_cycles % (self.config.peripheral_tick_interval as u64) == 0 {
                // Propagate peripherals
                let (interrupts, costs) = self.bus.tick_peripherals_fully();
                for c in costs {
                    self.total_cycles += c.cycles as u64;
                    if let Some(p) = self.bus.peripherals.get(c.index) {
                        for observer in &self.observers {
                            observer.on_peripheral_tick(&p.name, c.cycles);
                        }
                    }
                }
                for irq in interrupts {
                    self.cpu.set_exception_pending(irq);
                    tracing::debug!("Exception {} Pend", irq);
                }
            }

            // If we executed less than requested, it means the CPU wanted to exit early (e.g. branch/exception)
            // or we just finished the batch naturally. The loop will continue and check breakpoints/limits.
            if executed == 0 && current_batch > 0 {
                // If the CPU makes no progress but says Ok(0), we might need to investigate.
                // For now, assume it's valid (e.g. waiting for something).
                break;
            }
        }
        Ok(StopReason::StepDone)
    }

    fn step_single(&mut self) -> SimResult<StopReason> {
        self.step()?;
        Ok(StopReason::StepDone)
    }

    fn read_core_reg(&self, id: u8) -> u32 {
        self.cpu.get_register(id)
    }

    fn write_core_reg(&mut self, id: u8, val: u32) {
        self.cpu.set_register(id, val);
    }

    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>> {
        let mut data = Vec::with_capacity(len);
        for i in 0..len {
            let byte = self.bus.read_u8((addr as u64) + (i as u64))?;
            data.push(byte);
        }
        Ok(data)
    }

    fn write_memory(&mut self, addr: u32, data: &[u8]) -> SimResult<()> {
        for (i, byte) in data.iter().enumerate() {
            self.bus.write_u8((addr as u64) + (i as u64), *byte)?;
        }
        Ok(())
    }

    fn get_pc(&self) -> u32 {
        self.cpu.get_pc()
    }

    fn set_pc(&mut self, addr: u32) {
        self.cpu.set_pc(addr);
    }

    fn get_register_names(&self) -> Vec<String> {
        self.cpu.get_register_names()
    }

    fn get_cycle_count(&self) -> u64 {
        self.total_cycles
    }

    fn get_peripherals(&self) -> Vec<(String, u64, u64)> {
        self.bus
            .peripherals
            .iter()
            .map(|p| (p.name.clone(), p.base, p.size))
            .collect()
    }

    fn get_peripheral_descriptor(
        &self,
        name: &str,
    ) -> Option<labwired_config::PeripheralDescriptor> {
        use crate::peripherals::declarative::GenericPeripheral;
        let entry = self.bus.peripherals.iter().find(|p| p.name == name)?;
        let gen_p = entry.dev.as_any()?.downcast_ref::<GenericPeripheral>()?;
        // We need a way to get the descriptor from GenericPeripheral.
        // It's private currently. Let's make it public or add a getter.
        Some(gen_p.get_descriptor().clone())
    }

    fn reset(&mut self) -> SimResult<()> {
        self.cpu.reset(&mut self.bus)
    }

    fn snapshot(&self) -> snapshot::MachineSnapshot {
        self.snapshot()
    }

    fn restore(&mut self, snapshot: &snapshot::MachineSnapshot) -> SimResult<()> {
        self.apply_snapshot(snapshot.clone())
    }
}
