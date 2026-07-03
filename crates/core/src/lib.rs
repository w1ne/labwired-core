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
pub mod coverage;
pub mod cpu;
pub mod decoder;
pub mod fidelity;
pub mod interrupt;
pub mod memory;
pub mod metrics;
pub mod multi_core;
pub mod network;
pub mod pc_coverage;
pub mod peripherals;
pub mod physics;
pub mod runtime_snapshot;
pub mod sched;
pub mod signals;
pub mod sim_input;
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
    /// Unit-level reshape for engines that convert between source and
    /// destination data widths (STM32H5 GPDMA CTR1 PAM / SBX / DBX / DHX).
    /// `None` keeps the classic single-byte semantics.
    pub transform: Option<DmaUnitTransform>,
}

/// One source-data-unit -> destination-data-unit transform descriptor.
/// Semantics follow RM0481 §15 and are pinned by the DMA_DataHandling
/// HAL example's expected-result vectors (sim) and its on-board run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DmaUnitTransform {
    /// Source data width in bytes (1, 2 or 4).
    pub src_width: u8,
    /// Destination data width in bytes (1, 2 or 4).
    pub dst_width: u8,
    /// CTR1.PAM[0]: 0 = right-align zero-pad (narrow->wide) / left-truncate
    /// keep-LSBs (wide->narrow); 1 = sign-extend / right-truncate keep-MSBs.
    pub pam: u8,
    /// CTR1.SBX — exchange the two middle bytes of a word-width source.
    pub sbx: bool,
    /// CTR1.DBX — swap bytes within each destination half-word.
    pub dbx: bool,
    /// CTR1.DHX — swap the half-words of a word-width destination.
    pub dhx: bool,
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
    fn on_trace_event(&self, _event: labwired_hw_trace::TraceEvent) {}
    fn on_step_start(&self, _pc: u32, _opcode: u32) {}
    fn on_step_end(&self, _cycles: u32, _registers: &[u32]) {}
    fn on_memory_write(&self, _addr: u64, _old: u8, _new: u8) {}
    fn on_peripheral_tick(&self, _name: &str, _cycles: u32) {}
}

pub fn emit_trace_event(
    observers: &[Arc<dyn SimulationObserver>],
    event: labwired_hw_trace::TraceEvent,
) {
    for observer in observers {
        observer.on_trace_event(event.clone());
    }
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

    /// Bus-aware tick hook for peripherals that need to read or write the
    /// bus themselves (e.g. Easy DMA on RADIO). Default no-op.
    fn tick_with_bus(&mut self, _bus: &mut dyn Bus) {}

    /// True if this peripheral wants the bus to call `tick_with_bus`.
    /// Default false so the bus skips the swap dance for everyone else.
    fn needs_bus_tick(&self) -> bool {
        false
    }
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

    /// Phase 2B.1 (issue #192): event-driven peripheral scheduler hook.
    /// Default returns an empty result; peripherals that opt in
    /// (`uses_scheduler() == true`) override to interpret `event_token`
    /// via their own internal token enum and produce side-effects via
    /// the shared `EventResult` channel.
    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut sched::EventScheduler,
        _bus: &mut dyn Bus,
    ) -> sched::EventResult {
        sched::EventResult::default()
    }

    /// Phase 2B.1: synchronously notified when a subscribed clock domain
    /// changes rate. Implementations typically cancel in-flight events and
    /// reschedule at the new cadence. Default no-op.
    fn on_clock_change(
        &mut self,
        _domain: sched::ClockDomain,
        _new_hz: u64,
        _sched: &mut sched::EventScheduler,
    ) {
    }

    /// Phase 2B.1: when `true`, `Machine::step` skips this peripheral's
    /// legacy `tick()` walk and relies on the scheduler to drive it. Default
    /// `false` preserves existing per-cycle tick behaviour.
    fn uses_scheduler(&self) -> bool {
        false
    }

    /// Phase 2B.2 (issue #192): advance a scheduler-driven peripheral's lazy
    /// state to `tick_now` (the peripheral-tick index — CPU cycles divided by
    /// `peripheral_tick_interval`, the same quantum the legacy walk advanced
    /// one step per `tick()` call). Called by the bus immediately before an
    /// MMIO write observes the peripheral, so a frozen-then-strobed counter
    /// reads the up-to-date value. Default no-op; only peripherals that opt
    /// into the scheduler implement it.
    fn sync_to(&mut self, _tick_now: u64) {}

    /// Phase 2B.3a (issue #192): hand the bus any events this peripheral wants
    /// scheduled as a result of the MMIO write that just completed (e.g. a
    /// UART arming its TX interrupt). Each entry is `(delay_ticks, token)` —
    /// a delay in peripheral-tick units from "now" and an opaque token the
    /// peripheral interprets in its own `on_event`. The buffer is drained
    /// (cleared) by this call. A peripheral can't reach the scheduler from
    /// `write`, so this is the bootstrap path; `on_event` reschedules itself
    /// thereafter. Default empty — only write-scheduling peripherals override.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        Vec::new()
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

    /// Check whether a NVIC exception is still hardware-pending (ISPR bit set).
    ///
    /// Called by the CPU before dispatching an exception that is present in
    /// `pending_exceptions` but may have been software-cleared via NVIC ICPR
    /// (e.g. `NRFY_IRQ_PENDING_CLEAR`) while the ISR was still executing.
    /// Returns `true` for non-NVIC exceptions (num < 16) or when no NVIC is
    /// present — the conservative/backward-compatible default.
    fn is_nvic_irq_pending(&self, _exception_num: u32) -> bool {
        true
    }

    /// Plan 3: route a peripheral source ID to a cpu0 IRQ slot via a
    /// registered ESP32-S3 interrupt matrix peripheral. Default returns
    /// None for buses that don't model this.
    fn route_irq_source_to_cpu_irq(&self, _source_id: u32) -> Option<u8> {
        None
    }

    /// Bitmask of pending CPU IRQ slots the bus has aggregated for the given
    /// core (`core_id` 0 = PRO_CPU, 1 = APP_CPU). On ESP32-S3 the per-core
    /// interrupt matrix routes peripheral source IDs (and the crosscore_ipi
    /// FROM_CPU sources) to each core; the classic-ESP32 dual-core path
    /// delivers FROM_CPU IPIs via the DPORT interrupt matrix instead.
    /// Default returns 0; non-ESP32 buses don't model this.
    fn pending_cpu_irqs(&self, _core_id: u8) -> u32 {
        0
    }

    /// Plan 3: clear the pending bit for cpu IRQ `slot` on `core_id`.
    fn clear_cpu_irq_pending(&mut self, _core_id: u8, _slot: u8) {}

    /// ESP32-C3 (RISC-V) external-interrupt delivery: the level-sensitive
    /// bitmask of CPU interrupt lines (1..31) currently asserted, after the bus
    /// has routed asserted peripheral sources (and the SYSTEM FROM_CPU IPI
    /// registers) through the INTERRUPT_CORE0 matrix MAP registers. The RISC-V
    /// core ORs this into `mip` when deciding whether to trap, so a source that
    /// de-asserts (e.g. the FROM_CPU yield register cleared by the ISR) drops
    /// its line on the next tick — no latching. Default 0 for buses that don't
    /// model it (the line stays low and nothing changes).
    fn external_irq_lines(&self) -> u32 {
        0
    }

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
    ) -> Option<crate::peripherals::esp_xtensa_common::rom_thunks::RomThunkFn> {
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

    /// Phase 2B.1 (issue #192): event-driven peripheral scheduler. Active
    /// behaviour is gated behind the `event-scheduler` feature; the field
    /// is always present so other crates can borrow it without cfg gates.
    pub sched: sched::EventScheduler,
    /// Phase 2B.1: clock domain registry + observer fan-out. Wiring to
    /// real ESP32 register writes lands with the DPORT / RTC_CNTL
    /// migration PRs (design §12a).
    pub clocks: sched::ClockGraph,
    /// Cached bus index of the (single) RTC_CNTL peripheral, resolved at
    /// construction. `step()` drains the SW_SYS_RST latch every cycle; the
    /// pre-cache code walked the full ~40-peripheral list and downcast each
    /// to `RtcCntl` per cycle. With the index cached this collapses to one
    /// indexed access + one downcast. `None` for configs that don't register
    /// an RTC_CNTL peripheral (every non-ESP32-classic target).
    rtc_cntl_index: Option<usize>,
    /// Cached bus index of the FLASH peripheral (H5 layout). Resolved once at
    /// construction; `step()` drains pending FLASH ops (sector erase,
    /// bank-swap+reset) at clean instruction boundaries without walking the
    /// full peripheral list every cycle. `None` for configs with no FLASH
    /// peripheral on the bus (e.g. bare-bus unit tests).
    flash_index: Option<usize>,
    /// Cached bus index of the SCB peripheral (Cortex-M). Resolved once at
    /// construction; `step()` drains a pending SYSRESETREQ latch every cycle
    /// and, when set, reboots the CPU through the vector table via the
    /// existing `Machine::reset`. `None` for non-Cortex-M targets (no SCB on
    /// the bus), so the per-cycle drain short-circuits without a peripheral
    /// walk or downcast.
    scb_index: Option<usize>,
    /// Phase 2B.3b (issue #192): whether the one-time scheduler bootstrap has
    /// run. On the first `drain_scheduler_events`, peripherals with setup-time
    /// work (e.g. a UART with an RX stream attached before any MMIO write) get
    /// one chance to schedule their initial events. Always present; only read
    /// under the `event-scheduler` feature.
    #[allow(dead_code)]
    scheduler_bootstrapped: bool,
}

impl<C: Cpu> Machine<C> {
    /// Discover the drivable input channels on this machine (delegates to
    /// [`bus::SystemBus::list_inputs`]). See [`crate::sim_input`].
    pub fn list_inputs(&self) -> Vec<(String, crate::sim_input::InputChannel)> {
        self.bus.list_inputs()
    }

    /// Drive a simulated input channel to `value` (delegates to
    /// [`bus::SystemBus::set_input`]). The generic entry point an agent, the
    /// WASM bridge, or a test-script stimulus calls to steer an input device
    /// mid-run without knowing its concrete type.
    pub fn set_input(
        &mut self,
        channel: &str,
        value: f64,
    ) -> Result<(), crate::sim_input::SimInputError> {
        self.bus.set_input(channel, value)
    }
}

impl<C: Cpu> Machine<C> {
    pub fn new(cpu: C, bus: bus::SystemBus) -> Self {
        let rtc_cntl_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::rtc_cntl::RtcCntl>())
                .is_some()
        });
        let flash_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::flash::Flash>())
                .is_some()
        });
        let scb_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::scb::Scb>())
                .is_some()
        });
        Self {
            cpu,
            cpu_secondary: None,
            bus,
            observers: Vec::new(),
            breakpoints: HashSet::new(),
            last_breakpoint: None,
            total_cycles: 0,
            config: SimulationConfig::default(),
            sched: sched::EventScheduler::new(),
            clocks: sched::ClockGraph::new(),
            rtc_cntl_index,
            flash_index,
            scb_index,
            scheduler_bootstrapped: false,
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
            // 2b. Try extra CPU-visible memory windows (ESP32 IRAM/DROM, etc).
            if self
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
            {
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

        // Resolve the reset vector. Reset reads the initial (SP, PC) from the
        // vector table at VTOR, which defaults to 0 and aliases to the flash
        // base — correct for the common case (STM32/nRF/etc.). Some SoCs
        // prepend a second-stage bootloader: the RP2040 bootrom runs a 256-byte
        // stage-2 (boot2) blob from flash and only then enters the application
        // vector table at `flash_base + reset_vector_offset`. We don't execute
        // boot2 (flash is directly mapped), so when the flash-base vectors are
        // not valid, relocate to the declared post-stage-2 table — emulating
        // boot2's only observable effect.
        let flash_base = self.bus.flash.base_addr;
        if !self.bus.vector_pair_valid(flash_base) {
            let offset = self.bus.reset_vector_offset;
            let relocated = if offset != 0 {
                let table = flash_base + offset;
                if self.bus.vector_pair_valid(table) {
                    let sp = self.bus.read_u32(table)?;
                    let pc = self.bus.read_u32(table + 4)? & !1;
                    // Point VTOR at the relocated table so early exceptions
                    // (before firmware sets VTOR itself) vector correctly.
                    let _ = self.bus.write_u32(0xE000_ED08, table as u32);
                    self.cpu.set_sp(sp);
                    self.cpu.set_pc(pc);
                    true
                } else {
                    false
                }
            } else {
                false
            };

            // Last-resort fallback if the vector table is missing/zero.
            if !relocated && self.cpu.get_pc() == 0 {
                self.cpu.set_pc(image.entry_point as u32);
            }
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
        // Mirror the cycle count into the bus before the CPU executes, so
        // tick-time services can read "now": scheduler-driven peripheral sync
        // (event-scheduler) and the HC-SR04 echo-window timing (always). O(1) —
        // a single field write, not the per-peripheral walk this phase removed.
        self.bus.current_cycle = self.total_cycles;
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
                crate::peripherals::esp_xtensa_common::rom_thunks::APPCPU_BOOT_ADDR
                    .with(|s| s.take())
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

        // Phase 2B.1 (issue #192): event-driven peripheral scheduler.
        // With the `event-scheduler` flag OFF this block compiles out
        // entirely and behaviour matches pre-2B `main`. With the flag ON
        // and no peripheral opted in (`uses_scheduler() == false` for
        // everyone) the drain is a no-op — the legacy `tick()` walk
        // above still drives every peripheral until each migrates.
        #[cfg(feature = "event-scheduler")]
        self.drain_scheduler_events();

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

        // Cortex-M SCB system reset (AIRCR.SYSRESETREQ with the VECTKEY).
        // Firmware that asks for a reboot (e.g. a UDS ECUReset) writes
        // AIRCR and does not expect the store to return; on real silicon the
        // core restarts through the vector table. We drain the latch here, at
        // the same clean instruction boundary as RTC_CNTL — after the
        // AIRCR-writing store and any pending peripheral effects of this
        // instruction have been applied — then reuse the power-on reset
        // machinery so MSP/PC reload from vector[0]/vector[1] via the CPU
        // reset path. No-op on non-Cortex-M targets (no SCB on the bus).
        if self.drain_scb_reset_request() {
            self.reset()?;
            tracing::debug!("SCB SYSRESETREQ: CPU rebooted through vector table");
        }

        // H5 FLASH pending ops: sector erase fills flash with 0xFF; bank-swap
        // swaps the two 1 MB banks in the flash buffer then re-runs reset so
        // the CPU boots from the new bank-1 vector table. Also drained on the
        // batch/CLI run path (`Machine::run`), which executes cycle-accurately
        // when an H5 op-modeling FLASH is present so this fires per instruction.
        self.apply_pending_flash_op()?;

        Ok(())
    }

    /// Drain and apply the single pending H5 FLASH hardware operation, if any.
    ///
    /// The FLASH peripheral records at most one op per instruction (in a `Cell`);
    /// this helper must therefore run once per instruction so no op is lost. It
    /// is called from both `step()` and the `Machine::run` batch loop body. The
    /// run loop clamps its batch to 1 when `requires_cycle_accurate()` is true
    /// (which an H5 op-modeling FLASH forces), preserving the one-op-per-
    /// instruction invariant and the correct erase-before-program ordering.
    pub(crate) fn apply_pending_flash_op(&mut self) -> SimResult<()> {
        if let Some(op) = self.drain_flash_op() {
            use crate::peripherals::flash::h5;
            use crate::peripherals::flash::FlashOp;
            match op {
                FlashOp::EraseSector { bank, sector } => {
                    // OPT-IN read-while-write fidelity gate (H5 only, default
                    // off). On real STM32H563 silicon a bank cannot be fetched
                    // from while it is being erased — code running from that bank
                    // stalls and cannot make progress, so production flash
                    // routines run from SRAM. When the gate is on, an erase of
                    // the SAME physical bank the CPU is currently executing from
                    // is an unrecoverable access fault rather than a silent
                    // success. The bank comparison is SWAP_BANK-aware: both the
                    // BKSEL logical erase bank and PC's bank are mapped to a
                    // physical bank through the active swap state (see
                    // `Flash::rww_erase_violates`). Gate off ⇒ this branch is
                    // skipped entirely and the erase proceeds as before.
                    let pc = self.cpu.get_pc() as u64;
                    let in_flash =
                        (h5::FLASH_BASE..h5::FLASH_BASE + 2 * h5::BANK_SIZE).contains(&pc);
                    if let Some(flash) = self.flash_peripheral() {
                        if flash.h5_rww_enabled()
                            && in_flash
                            && flash.rww_erase_violates(bank, pc - h5::FLASH_BASE)
                        {
                            let phys = flash.physical_bank_of_offset(pc - h5::FLASH_BASE);
                            return Err(SimulationError::Other(format!(
                                "flash RWW violation: erase of bank {phys} while executing \
                                 from bank {phys} (PC={pc:#010x}) — run the flash routine \
                                 from SRAM"
                            )));
                        }
                    }
                    let offset = (bank as u64) * h5::BANK_SIZE + (sector as u64) * h5::SECTOR_SIZE;
                    self.bus.flash.fill(offset, h5::SECTOR_SIZE, 0xFF);
                    tracing::debug!(
                        "FLASH EraseSector bank={bank} sector={sector} offset={offset:#010x}"
                    );
                }
                FlashOp::SwapAndReset => {
                    // Swap the two architectural 1 MiB (0x100000) banks. The
                    // H563 flash buffer is sized to exactly 2 * BANK_SIZE by the
                    // chip yaml (`size: "2MiB"`), so the same BANK_SIZE used by
                    // EraseSector above also bounds the swap — keeping erase and
                    // swap on one consistent bank-size notion (real silicon:
                    // bank 2 @ 0x08100000). swap_banks returns false if the
                    // buffer is not exactly two banks; debug-assert that here so
                    // a mis-sized chip yaml fails loudly in tests.
                    let swapped = self.bus.flash.swap_banks(h5::BANK_SIZE);
                    debug_assert!(
                        swapped,
                        "SWAP_BANK: flash buffer ({} bytes) is not 2 * BANK_SIZE ({}); \
                         check the chip yaml flash size uses binary (MiB) units",
                        self.bus.flash.data.len(),
                        2 * h5::BANK_SIZE
                    );
                    // Record the swap so the RWW bank mapping reflects the new
                    // active view (the physical second bank now answers at
                    // 0x08000000). No-op for the gate-off path beyond a bool.
                    if let Some(flash) = self.flash_peripheral_mut() {
                        flash.mark_swapped();
                    }
                    tracing::debug!("FLASH SwapAndReset: banks swapped, resetting CPU");
                    self.reset()?;
                }
            }
        }
        Ok(())
    }

    /// Phase 2B.1/2B.3a (issue #192): advance the scheduler and fire every due
    /// peripheral event. Called from both `step()` and the batch run loop so
    /// neither path silently strands a scheduler-driven peripheral.
    ///
    /// The scheduler runs in peripheral-tick units (`total_cycles /
    /// peripheral_tick_interval`) — the same quantum the legacy walk and
    /// `sync_to` use — so deadlines are interval-agnostic. Write-context
    /// schedule requests the bus buffered during this step's MMIO writes
    /// (`pending_schedule`) are enqueued first: a peripheral can't reach the
    /// scheduler from `write`, so it hands `(delay_ticks, token)` to the bus
    /// and we convert to an absolute deadline here.
    #[cfg(feature = "event-scheduler")]
    fn drain_scheduler_events(&mut self) {
        // One-time bootstrap: give every scheduler-driven peripheral a chance
        // to schedule events that arise from *setup* rather than an MMIO write
        // (e.g. a UART with an RX stream attached before firmware runs).
        if !self.scheduler_bootstrapped {
            self.scheduler_bootstrapped = true;
            for idx in 0..self.bus.peripherals.len() {
                if self.bus.peripherals[idx].dev.uses_scheduler() {
                    for (delay, token) in self.bus.peripherals[idx].dev.take_scheduled_events() {
                        self.bus.pending_schedule.push((idx, delay, token));
                    }
                }
            }
        }
        let interval = (self.config.peripheral_tick_interval as u64).max(1);
        self.sched.advance_to(self.total_cycles / interval);
        let now = self.sched.now();
        for (idx, delay, token) in std::mem::take(&mut self.bus.pending_schedule) {
            let gen = self
                .bus
                .peripherals
                .get(idx)
                .map(|p| p.generation)
                .unwrap_or(0);
            self.sched.schedule(now + delay, idx as u32, token, gen);
        }
        let generations = self.bus.peripheral_generations();
        let due = self.sched.drain_due(&generations);
        for ev in due {
            let idx = ev.peripheral_idx as usize;
            let Some(entry) = self.bus.peripherals.get_mut(idx) else {
                continue;
            };
            // Swap the peripheral out so we can pass `&mut self.bus` into
            // `on_event` without holding two simultaneous mutable borrows.
            // Same dance the bus uses for `tick_with_bus`.
            let placeholder: Box<dyn Peripheral> =
                Box::new(crate::peripherals::stub::StubPeripheral::new(0));
            let mut dev = std::mem::replace(&mut entry.dev, placeholder);
            let result = dev.on_event(ev.event_token, &mut self.sched, &mut self.bus);
            self.bus.peripherals[idx].dev = dev;
            // Phase 2B.3b: a level-triggered peripheral re-arms its own event
            // (same token) while it has active work. We own the (idx,
            // generation) the scheduler needs, so we do it here.
            if let Some(delay) = result.reschedule_delay {
                let gen = self.bus.peripherals[idx].generation;
                let deadline = self.sched.now() + delay;
                self.sched
                    .schedule(deadline, idx as u32, ev.event_token, gen);
            }
            self.apply_event_result(idx, result);
        }
    }

    /// Phase 2B.1 (issue #192): fan out the side-effects produced by a
    /// `Peripheral::on_event` handler. Mirrors the post-`tick()` fan-out
    /// in `tick_peripherals_phase1`: IRQ pend, system exception, mmio
    /// writes, PPI fired_events globalisation, DMA execute. No peripheral
    /// opts into the scheduler in 2B.1, so this code only runs when the
    /// `event-scheduler` feature is on AND a peripheral overrides
    /// `uses_scheduler()` in a later migration PR.
    #[cfg(feature = "event-scheduler")]
    fn apply_event_result(&mut self, peripheral_idx: usize, result: sched::EventResult) {
        let base = self.bus.peripherals[peripheral_idx].base as u32;
        let mut fallthrough: Vec<u32> = Vec::new();
        if let Some(irq) = result.raise_irq {
            self.bus.pend_irq_for_event(irq, &mut fallthrough);
        }
        // Phase 2B.3b: pend the peripheral's *own* configured NVIC line — the
        // event-path equivalent of the legacy `tick()` returning `irq: true`.
        if result.raise_own_irq {
            if let Some(irq) = self.bus.peripherals[peripheral_idx].irq {
                self.bus.pend_irq_for_event(irq, &mut fallthrough);
            }
        }
        for irq in &result.explicit_irqs {
            self.bus.pend_irq_for_event(*irq, &mut fallthrough);
        }
        // Phase 2B.3b: route DMA signals exactly as the legacy tick path does.
        if !result.dma_signals.is_empty() {
            let source_name = self.bus.peripherals[peripheral_idx].name.clone();
            for sig in &result.dma_signals {
                self.bus.route_dma_signal(&source_name, *sig);
            }
        }
        if let Some(exc) = result.system_exception {
            self.cpu.set_exception_pending(exc);
        }
        for irq in fallthrough {
            self.cpu.set_exception_pending(irq);
        }
        for (addr, val) in result.mmio_writes {
            if let Err(e) = self.bus.write_u32(addr as u64, val) {
                tracing::warn!("on_event mmio_write 0x{addr:08X} = 0x{val:08X} failed: {e:?}");
            }
        }
        // PPI fan-out: globalise event offsets to absolute bus addresses and
        // route through any peripheral that overrides `route_ppi_events`.
        if !result.fired_events.is_empty() {
            let fired_global: Vec<u32> = result
                .fired_events
                .iter()
                .map(|off| base.wrapping_add(*off))
                .collect();
            let mut pending_tasks: Vec<u32> = Vec::new();
            for p in self.bus.peripherals.iter_mut() {
                pending_tasks.extend(p.dev.route_ppi_events(&fired_global));
            }
            for task_addr in pending_tasks {
                if let Err(e) = self.bus.write_u32(task_addr as u64, 1) {
                    tracing::warn!("on_event PPI task 0x{task_addr:08X} failed: {e:?}");
                }
            }
        }
        if !result.dma_requests.is_empty() {
            if let Err(e) = self.bus.execute_dma(&result.dma_requests) {
                tracing::warn!("on_event execute_dma failed: {e:?}");
            }
        }
    }

    /// Returns true (and clears the latch) if the registered RTC_CNTL
    /// peripheral has a pending software-system-reset request. Used by
    /// `step()` to honor OPTIONS0 bit 31 writes at a clean instruction
    /// boundary. Uses the cached `rtc_cntl_index` resolved at construction
    /// — non-ESP32-classic configs (no RTC_CNTL on the bus) short-circuit
    /// to `false` without touching the peripheral vector at all.
    fn drain_rtc_cntl_reset_request(&self) -> bool {
        let Some(idx) = self.rtc_cntl_index else {
            return false;
        };
        let Some(p) = self.bus.peripherals.get(idx) else {
            return false;
        };
        p.dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::rtc_cntl::RtcCntl>())
            .map(|rtc| rtc.drain_reset_request())
            .unwrap_or(false)
    }

    /// Returns true (and clears the latch) if the registered SCB peripheral
    /// has a pending SYSRESETREQ — an AIRCR write with the correct VECTKEY
    /// and the SYSRESETREQ bit set (`Scb::write_reg`, offset 0x0C). Used by
    /// `step()` to honor a firmware-requested system reset at a clean
    /// instruction boundary. Uses the cached `scb_index` resolved at
    /// construction — non-Cortex-M configs (no SCB on the bus) short-circuit
    /// to `false` without touching the peripheral vector at all.
    pub fn drain_scb_reset_request(&self) -> bool {
        let Some(idx) = self.scb_index else {
            return false;
        };
        let Some(p) = self.bus.peripherals.get(idx) else {
            return false;
        };
        p.dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::scb::Scb>())
            .map(|scb| scb.drain_reset_request())
            .unwrap_or(false)
    }

    /// Drain a pending FLASH hardware operation recorded by the H5 FLASH
    /// peripheral (sector erase or bank-swap+reset). Returns `None` for
    /// configs that have no FLASH peripheral on the bus. The caller is
    /// responsible for applying the returned op to `bus.flash` and issuing
    /// a CPU reset when required.
    fn drain_flash_op(&self) -> Option<crate::peripherals::flash::FlashOp> {
        let idx = self.flash_index?;
        let p = self.bus.peripherals.get(idx)?;
        p.dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::flash::Flash>())
            .and_then(|f| f.drain_pending_op())
    }

    /// Borrow the H5 FLASH peripheral, if one is on the bus. Used by the
    /// read-while-write gate to query the swap state / bank mapping.
    fn flash_peripheral(&self) -> Option<&crate::peripherals::flash::Flash> {
        let idx = self.flash_index?;
        self.bus
            .peripherals
            .get(idx)?
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::flash::Flash>())
    }

    /// Mutably borrow the FLASH peripheral, if one is on the bus. Used to record
    /// the applied SWAP_BANK so the RWW bank mapping tracks the active view.
    fn flash_peripheral_mut(&mut self) -> Option<&mut crate::peripherals::flash::Flash> {
        let idx = self.flash_index?;
        self.bus
            .peripherals
            .get_mut(idx)?
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<crate::peripherals::flash::Flash>())
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
            // Mirror the cycle count before the batch so MMIO writes inside it
            // (and tick-time services) can read "now". The batch is bounded by
            // `peripheral_tick_interval`, so intra-batch staleness is < one tick.
            self.bus.current_cycle = current_cycles;
            let tick_interval = self.config.peripheral_tick_interval as u64;
            let remaining_until_tick = (tick_interval - (current_cycles % tick_interval)) as u32;

            let mut current_batch = if let Some(limit) = max_steps {
                remaining_until_tick.min(limit - steps)
            } else {
                remaining_until_tick
            };

            // Cycle-accurate buses (HC-SR04, IO-Link, H5 op-modeling FLASH) must
            // execute one instruction per batch so per-instruction services —
            // notably the H5 FLASH pending-op drain below — fire on every
            // instruction. Without this clamp the FLASH op would be recorded in
            // the peripheral cell but applied at most once per tick interval (or
            // not at all), losing all but the last op and breaking erase-before-
            // program ordering.
            if self.bus.requires_cycle_accurate() {
                current_batch = current_batch.min(1);
            }

            // Breakpoints are only checked at batch boundaries (top of loop). If
            // any breakpoint is set, clamp the batch to one instruction so a
            // breakpoint whose PC lies inside a batch is caught at exactly that
            // PC instead of being executed past and noticed only at the next
            // boundary (the GDB "continue never stops" bug). This per-instruction
            // cost applies ONLY while breakpoints are set, i.e. under a debugger,
            // so the no-breakpoint hot path is unaffected.
            if !self.breakpoints.is_empty() {
                current_batch = current_batch.min(1);
            }

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

            #[cfg(feature = "event-scheduler")]
            self.drain_scheduler_events();

            // Apply any pending H5 FLASH op recorded by the instructions just
            // executed. On a cycle-accurate bus the batch is clamped to 1 above,
            // so this runs per instruction (matching `step()`); this is the path
            // the CLI test runner and `Machine::run` take, where the op would
            // otherwise never be applied.
            self.apply_pending_flash_op()?;

            // Honor a firmware-requested system reset (AIRCR SYSRESETREQ with
            // VECTKEY) latched by the instructions just executed. `step()` drains
            // this on every instruction boundary; the batched `run` path must do
            // the same on every batch return or the reboot never fires.
            if self.drain_scb_reset_request() {
                self.reset()?;
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
