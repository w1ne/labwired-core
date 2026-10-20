#![allow(clippy::manual_is_multiple_of)]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod analog;
pub mod boot;
pub mod bus;
pub mod census;
pub mod config;
pub mod console;
pub mod cosim;
pub mod coverage;
pub mod cpu;
pub mod cycle_clock;
pub mod debug;
pub mod decoder;
pub mod fault_verdict;
pub mod fidelity;
pub mod hashers;
pub mod host_time;
pub mod inspect;
pub mod interrupt;
pub mod logic_capture;
pub mod machine;
pub mod memory;
pub mod metrics;
pub mod multi_core;
pub mod network;
pub mod pc_coverage;
pub mod peripheral;
pub mod peripheral_log;
pub mod peripherals;
pub mod physics;
pub mod pins;
pub mod plugin;
pub mod power;
pub mod profile;
pub mod runtime_snapshot;
pub mod sched;
pub mod sd_hle;
pub mod session;
pub mod signals;
pub mod sim_input;
pub mod snapshot;
pub mod stack_paint;
pub mod system;
#[doc(hidden)]
pub mod test_support;
pub mod trace;
pub mod uds_evidence;
pub mod vfi;
pub mod world;

pub use config::SimulationConfig;
pub use cycle_clock::CycleClock;
pub use host_time::HostTimeMode;
pub use machine::{
    AdvanceLimits, AdvanceReport, AdvanceRequest, AdvanceStop, BatchPolicy, BreakpointPolicy,
    IdlePolicy,
};
pub use peripheral::{
    DmaDirection, DmaRequest, DmaRoute, DmaUnitTransform, MmioAccessClass, Peripheral,
    PeripheralResetCause, PeripheralTickResult, DMA_SIGNAL_RX, DMA_SIGNAL_TX,
};

use std::any::Any;
use std::sync::Arc;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Arch {
    Arm,
    RiscV,
    XtensaLx7,
    /// AVR8 (ATmega328P-class).
    Avr,
    /// PIC24 / dsPIC33 (dsPIC33FJ32MC204-class, M0 subset).
    Pic24,
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

/// Trait for observing simulation events in a modular way.
///
/// Every CPU core MUST emit, for each retired instruction, in this order:
///
///   1. `on_step_start(pc, opcode)` — before execution, `pc` is the address
///      being executed and `opcode` its raw little-endian encoding. Native
///      Cortex-A delivers this notification after execution, in the same event
///      order, because zero-retirement exception entry must emit no event.
///   2. `TraceEvent::InstructionRetired { pc, opcode }` via [`emit_trace_event`].
///   3. `on_step_end(cycles, registers)` — after execution.
///
/// A core may skip all three when `observers.is_empty()` (building the
/// register view is not free), but it may NOT skip them for any other reason:
/// in particular a JIT/compiled-block fast path must fall back to the
/// interpreter while anyone is observing, or the trace silently loses the
/// instructions that matter most.
///
/// `registers` is arch-specific in its leading entries, but the **last two
/// slots are standardized** so that arch-agnostic consumers work everywhere:
///
/// ```text
///   [ .. architectural registers, ISA numbering .. , SP , PC ]
/// ```
///
/// SP and PC are repeated in those trailing slots even when they already
/// appear in the arch block (ARM's r13/r15, RISC-V's x2). The duplication is
/// deliberate: it means a consumer never has to know which core produced a
/// trace to find the two values it almost always wants. See
/// [`SP_FROM_END`]/[`PC_FROM_END`], and `tests/cpu_trace_conformance.rs`,
/// which runs every core and fails if one drifts from any of the above.
pub trait SimulationObserver: std::fmt::Debug + Send + Sync {
    fn on_simulation_start(&self) {}
    fn on_simulation_stop(&self) {}
    fn on_trace_event(&self, _event: labwired_hw_trace::TraceEvent) {}
    fn on_step_start(&self, _pc: u32, _opcode: u32) {}
    fn on_step_end(&self, _cycles: u32, _registers: &[u32]) {}
    fn on_memory_write(&self, _addr: u64, _old: u8, _new: u8) {}
    fn on_peripheral_tick(&self, _name: &str, _cycles: u32) {}
    /// The machine is about to step CPU core `core` (0 = the primary). Sent
    /// only by multi-core machines, before a different core steps and again
    /// when the primary resumes, so an observer that links one instruction to
    /// the next can keep one chain per core.
    fn on_core_switch(&self, _core: usize) {}
}

/// Offset of the standardized PC slot from the end of an `on_step_end`
/// register slice. See [`SimulationObserver`].
pub const PC_FROM_END: usize = 1;
/// Offset of the standardized SP slot from the end of an `on_step_end`
/// register slice. See [`SimulationObserver`].
pub const SP_FROM_END: usize = 2;

/// SP and PC out of an `on_step_end` register slice, whatever core produced
/// it. Returns `None` for a slice too short to carry the standard trailer.
pub fn trace_sp_pc(registers: &[u32]) -> Option<(u32, u32)> {
    let n = registers.len();
    if n < SP_FROM_END {
        return None;
    }
    Some((registers[n - SP_FROM_END], registers[n - PC_FROM_END]))
}

pub fn emit_trace_event(
    observers: &[Arc<dyn SimulationObserver>],
    event: labwired_hw_trace::TraceEvent,
) {
    for observer in observers {
        observer.on_trace_event(event.clone());
    }
}

pub(crate) use cpu::{advance_batch_cycle, default_step_batch, default_step_batch_with};
pub use cpu::{Cpu, CpuJitStats, SecondaryExecutionState};

/// Trait representing the system bus
/// Verdict from [`Bus::check_fetch_permission`] — whether an instruction fetch
/// at a given PC is allowed by a memory-protection unit the bus models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchPermission {
    /// No protection unit objects (the default for every bus).
    Allowed,
    /// Blocked, and the unit raised an interrupt the firmware's own handler
    /// will take. The core substitutes a NOP for the blocked fetch so the trap
    /// is taken at the next instruction boundary — the same one-instruction
    /// skid the ESP32-C3's asynchronous PMS fault has on silicon, which is why
    /// IDF reads the violating address out of registers instead of `mepc`.
    DeniedFaultRaised,
    /// Blocked, but nothing can deliver the fault to the firmware. The core
    /// stops with `MemoryViolation` rather than executing from a region the
    /// hardware would not have fetched from.
    DeniedUndeliverable,
}

/// Runs `f` with the device-transfer bypass set, restoring the previous state.
/// Nesting-safe with itself; it must NOT be nested inside a scope that sets
/// only ONE of the two gates (the restore writes both — the getter is the OR
/// of the policy and watch bypasses, and no current one-gate scope calls a
/// device engine synchronously). Device engines wrap their guest-memory
/// transfers in this; CPU-visible writes must NOT be wrapped.
pub fn with_device_write_bypass<R>(bus: &mut dyn Bus, f: impl FnOnce(&mut dyn Bus) -> R) -> R {
    let prev = bus.device_write_bypass();
    bus.set_device_write_bypass(true);
    let r = f(bus);
    bus.set_device_write_bypass(prev);
    r
}

pub trait Bus {
    /// ARM application-core interrupt levels, including deassertion.
    fn arm_irq_fiq_levels(&self) -> Option<(bool, bool)> {
        None
    }
    /// Stable allocation-unique physical code identity. Native ARM execution
    /// refuses buses that cannot supply one.
    fn arm_code_identity(&self) -> Option<u64> {
        None
    }
    fn arm_cp15_read(&self, register_key: u32, wide: bool) -> SimResult<u64> {
        Err(SimulationError::NotImplemented(format!(
            "ARM CP15 read {register_key:#x} (wide={wide})"
        )))
    }
    fn arm_cp15_write(&mut self, register_key: u32, wide: bool, _value: u64) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "ARM CP15 write {register_key:#x} (wide={wide})"
        )))
    }
    fn next_cpu_interrupt_deadline_cycle(&self) -> Option<u64> {
        None
    }
    /// An eight-byte transaction must remain one transaction. In particular,
    /// splitting an MMIO access can duplicate or consume register side effects.
    fn read_u64(&self, addr: u64) -> SimResult<u64> {
        Err(SimulationError::NotImplemented(format!(
            "eight-byte bus read at {addr:#x}"
        )))
    }
    fn write_u64(&mut self, addr: u64, _value: u64) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "eight-byte bus write at {addr:#x}"
        )))
    }
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
    /// True while a device-originated transfer (DMA engine, host RTT write)
    /// bypasses the CPU-store gates (`cpu_writes` policy, debug watches).
    /// Default: no bypass. See [`with_device_write_bypass`].
    fn device_write_bypass(&self) -> bool {
        false
    }
    /// Set the device-transfer bypass. Implementations that own gates must
    /// honour this; default is a no-op.
    fn set_device_write_bypass(&mut self, _on: bool) {}
    /// Run `f` on the peripheral with id `name` and let the bus see what it
    /// changed (its legacy-walk membership, the events it wants scheduled) —
    /// how one peripheral's bus-aware pass reaches another's state (a GDMA
    /// feeding a GP-SPI). `false` when there is no such peripheral or the bus
    /// has no peripherals. Default: none.
    fn with_peripheral_mut(
        &mut self,
        _name: &str,
        _f: &mut dyn FnMut(&mut dyn Peripheral),
    ) -> bool {
        false
    }
    /// Resolve a configured GPIO label and read its output latch.
    fn read_gpio_output_by_label(&self, _pin: &str) -> Option<bool> {
        None
    }

    /// Whether a memory-mapped peripheral window covers `addr`. A CPU that
    /// mirrors its on-chip registers to a bus-side model (the AVR USART host)
    /// asks this before forwarding, instead of swallowing a refused access.
    /// Default `false`: a bus with no such windows.
    fn has_mmio_window(&self, _addr: u64) -> bool {
        false
    }

    /// Byte TX readiness after bus routing/clock admission. Unsupported
    /// endpoints return None; supported but blocked endpoints return false.
    fn dma_tx_request(&self, _addr: u64) -> Option<bool> {
        None
    }

    /// Accept one TX byte through the bus's normal MMIO bookkeeping. False
    /// means not ready, not a successful beat. Default explicit refusal.
    fn try_dma_write_tx_u8(&mut self, _addr: u64, _value: u8) -> SimResult<bool> {
        Err(SimulationError::NotImplemented(
            "bus has no admitted byte TX DMA endpoint".into(),
        ))
    }

    /// Admit an extent for a RAM-only DMA profile without reaching through
    /// the bus's concrete type. Mapped targets must be primary RAM, never
    /// MMIO (including aliases), flash or extra mappings. Unmapped addresses
    /// may be admitted so the actual read/write reports the transfer fault;
    /// this is not a promise that the access will succeed. Default refusal.
    fn admits_ram_only_dma_extent(&self, _start: u64, _length: u64) -> bool {
        false
    }

    /// A CPU-owned switch that disables every internal pull-up of the chip's
    /// pin ports (AVR `MCUCR.PUD`, ATmega328P datasheet §14.4.1). The CPU
    /// calls it when the bit changes; the bus hands it to each port
    /// ([`crate::pins::PinPort::set_pull_ups_disabled`]). Default: a bus with
    /// no pin ports.
    fn set_pull_ups_disabled(&mut self, _disabled: bool) {}

    /// Is an instruction fetch at `pc` permitted by a memory-protection unit
    /// the bus models? Called by the core only when its 256-byte fetch window
    /// does not already cover `pc`, i.e. once per window refill. That is exact
    /// rather than approximate: the window is 256-byte aligned and at most 256
    /// bytes long, while ESP32-C3 PMS split lines are 512-byte aligned, so a
    /// window can never straddle a permission boundary.
    ///
    /// Default `Allowed` — buses without a protection unit are unchanged.
    fn check_fetch_permission(&mut self, _pc: u64) -> FetchPermission {
        FetchPermission::Allowed
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

    /// Re-derive the routed CPU-interrupt levels after an interrupt dispatch
    /// cleared the slots it took, once per dispatch.
    ///
    /// `clear_cpu_irq_pending` clears a ROUTED bit, not the peripheral source
    /// behind it. The source is level-sensitive and generally still asserting —
    /// the firmware ISR de-asserts it later with an INT_CLR write — so the
    /// routed bit has to come straight back. Until the walk-free ESP32-S3 path
    /// landed that happened implicitly: the per-cycle aggregation rebuilt the
    /// whole routed bitmap from the live source levels on the very next tick.
    /// A bus that no longer aggregates per cycle must re-derive HERE instead,
    /// or a level the guest can read back (`RSR.INTERRUPT` on Xtensa is
    /// `pending_cpu_irqs` OR-ed in) reads zero for the whole ISR.
    ///
    /// Called once after the dispatch clear loop, not per slot. Default no-op:
    /// on an NVIC bus the pending bit is owned by the NVIC, and on a bus that
    /// still aggregates per cycle the next tick does this anyway.
    fn resettle_cpu_irq_levels(&mut self) {}

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

    /// Event-scheduler Gap-#1 hook: `true` iff an MMIO write executed since the
    /// last `drain_scheduler_events` armed a peripheral event that has not yet
    /// been moved into the scheduler heap (it is sitting in `pending_schedule`).
    /// A CPU batch loop polls this after each interpreted instruction while the
    /// tick interval is widened (> 1) so it can END the batch on the arming
    /// write — the just-armed event is then enqueued by the post-batch drain and
    /// the NEXT batch's `next_event_deadline` clamp delivers it at its exact
    /// cycle, instead of the batch overrunning the deadline. O(1); default false
    /// for buses that don't model the scheduler.
    #[cfg(feature = "event-scheduler")]
    fn has_pending_schedule(&self) -> bool {
        false
    }

    /// Earliest absolute cycle among events sitting in `pending_schedule`
    /// (not yet on the scheduler heap). Used by the CPU batch loop to **clamp**
    /// the remaining batch to that deadline instead of ending on the first arm
    /// (far-future timers would otherwise force ~tens-of-instruction batches).
    /// Fidelity: we still never retire past the deadline without a drain.
    /// Default `None`.
    #[cfg(feature = "event-scheduler")]
    fn earliest_pending_deadline(&self) -> Option<u64> {
        None
    }

    /// The bus's mirrored "current cycle" (what lazy peripherals read through the
    /// shared `CycleClock`). A CPU batch loop reads this once at batch entry to
    /// learn the batch-start cycle, then republishes the EXACT cycle before each
    /// interpreted instruction via [`Self::publish_cycle`] so a mid-batch MMIO
    /// read of a lazily-derived counter sees `batch_start + retired` — the same
    /// value interval-1 would show — instead of the stale batch-start value.
    /// Default 0 for buses that don't model a cycle clock.
    fn current_cycle(&self) -> u64 {
        0
    }

    /// Republish the shared `CycleClock` to `cycle` (see [`Self::current_cycle`]).
    /// Called per interpreted instruction while the tick interval is widened, so
    /// the cost is a single relaxed atomic store on the hot path. Default no-op.
    fn publish_cycle(&mut self, _cycle: u64) {}

    /// Advance the published cycle by `delta`.
    ///
    /// `step_batch` republishes the live cycle once per retired instruction
    /// (issue #842), and did it as `publish_cycle(current_cycle() + delta)` --
    /// TWO vtable dispatches per instruction on a `&mut dyn Bus`. One method
    /// is one dispatch. The default composes the old pair so any Bus that does
    /// not override it behaves exactly as before.
    ///
    /// None of the three is cfg-gated: the defaults are constants and the
    /// `SystemBus` overrides touch only ungated state (`current_cycle`,
    /// `set_current_cycle`), so both worlds can compile them. The CALLERS stay
    /// gated -- that is where the feature decides behaviour.
    fn advance_cycle(&mut self, delta: u64) {
        let now = self.current_cycle();
        self.publish_cycle(now + delta);
    }

    /// This bus's `peripheral_tick_interval` — how many cycles one
    /// tick-equivalent of peripheral work covers. A scheduler-driven model that
    /// paces output over time (the shared `Uart` and its RX streams) reads it
    /// to size its own service cadence, so it wakes once per interval and
    /// replays that many tick-equivalents instead of demanding a wakeup every
    /// cycle. Returning `1` (the default, and what a cycle-accurate bus
    /// reports) reproduces the per-cycle cadence exactly.
    ///
    /// NOT cfg-gated, deliberately: the default body is the constant `1`, and
    /// leaving it available in both worlds lets a scheduler-driven model read
    /// its own poll cadence with no `#[cfg]` pair at the call site. That is
    /// three conditional-compilation sites deleted per model, on the surface
    /// `event_scheduler_cfg_ratchet` exists to shrink.
    fn peripheral_tick_interval(&self) -> u32 {
        1
    }

    /// Plan 2: deliver a coherent 32-bit value to peripherals after the
    /// four byte writes that compose a write_u32 have been dispatched.
    /// Default: no-op for buses that don't route to peripherals.
    fn notify_word_write(&mut self, _addr: u64, _value: u32) -> SimResult<()> {
        Ok(())
    }

    /// The bus's shared push-mode logic tap, when it carries one (a cheap
    /// `Arc` clone). CPU batch loops use it to advance the tap clock once per
    /// retired instruction while push capture is armed; `None` (the default)
    /// means no tap and no per-instruction work.
    fn logic_tap(&self) -> Option<logic_capture::LogicTap> {
        None
    }

    /// `true` when this bus cannot run a widened tick without losing a timed
    /// waveform (HC-SR04, flash-ops, GPIO timing devices). CPU JIT gates use
    /// this instead of downcasting to `SystemBus`. Default `false`.
    fn requires_cycle_accurate(&self) -> bool {
        false
    }

    /// Commit a coalesced RISC-V store-spin loop to ordinary flat RAM: write
    /// `value` at `addr` once and account for `stores` RAM writes, as if the
    /// interpreter had executed every elided `sw`. Returns `false`, having
    /// touched nothing, when this bus cannot prove the store would be a plain
    /// RAM write — then the caller interprets normally.
    ///
    /// A capability on the bus rather than an `as_any_mut()` +
    /// `downcast_mut::<SystemBus>()` reach from the CPU. The coalescer asks
    /// ONE question ("may these stores collapse into one?") and only the bus
    /// knows the answer (protection unit, observers, RAM window). Default
    /// `false`: a bus that does not model this never coalesces, which is
    /// exactly what the failed downcast used to mean.
    fn commit_ram_store_spin(&mut self, _addr: u32, _value: u32, _stores: u64) -> bool {
        false
    }

    /// Commit a coalesced Xtensa store-spin loop to a plain-memory peripheral
    /// window (IRAM/DRAM `RamPeripheral`): perform the ONE real `write_u32`
    /// of `value` at `addr` through the normal route, then account for
    /// `elided` further accesses the loop would have made. `Ok(false)`,
    /// having touched nothing, when the winning route is not plain memory or
    /// anything observes the bus.
    ///
    /// Same reason as [`Self::commit_ram_store_spin`]: a capability, not a
    /// downcast to `SystemBus`. Default `Ok(false)` (never coalesce).
    fn commit_plain_memory_store_spin(
        &mut self,
        _addr: u32,
        _value: u32,
        _elided: u64,
    ) -> SimResult<bool> {
        Ok(false)
    }

    /// `true` when a FLASH on this bus records hardware operations as pending
    /// ops at all (H5 erase/bank-swap, U5 page erase) — a STATIC property,
    /// unlike [`Self::has_pending_flash_op`].
    ///
    /// The JIT safety gate needs this one. A compiled block retires many
    /// instructions WITHOUT the per-instruction probe the interpreter batch
    /// does, so a bus that can record an op has to keep falling back — exactly
    /// as it did while [`Self::requires_cycle_accurate`] carried this duty.
    /// Default `false`.
    fn models_flash_ops(&self) -> bool {
        false
    }

    /// `true` when a FLASH operation (H5 erase/bank-swap, U5 page erase) was
    /// recorded by the instruction that just retired and is waiting for
    /// `Machine::apply_pending_flash_op` to drain it.
    ///
    /// The Cortex-M batch loop probes this after each instruction and ENDS the
    /// batch at the recording write, so the drain lands on exactly that
    /// instruction. That replaces pinning the whole run to quantum 1 via
    /// [`Self::requires_cycle_accurate`], which cost ~15x on H5/U5 boards
    /// because it also stops the hot-loop fast path from ever engaging.
    ///
    /// On the trait, not behind a concrete-type cast, for the same reason
    /// `requires_cycle_accurate` is: this runs PER INSTRUCTION on those buses. Implementations gate the scan behind their own cached bool, so
    /// every other bus pays one predictable false. Default `false`.
    fn has_pending_flash_op(&self) -> bool {
        false
    }

    /// Cycles until SysTick would raise exception 15. Default `None` (no
    /// SysTick, or it cannot fire). Cortex-M JIT uses this to refuse a
    /// compiled block that would skip the countdown edge.
    fn systick_ticks_until_fire(&self) -> Option<u64> {
        None
    }

    /// Advance SysTick by `n` cycles after a compiled block that did not wrap.
    /// Scheduler-mode SysTick is driven by the cycle clock bump; this is the
    /// legacy-walk path. Default no-op.
    fn systick_consume_cycles(&mut self, _n: u64) {}

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

    /// Append bytes to the semihosting stream. Default no-op: only `SystemBus`
    /// has the sink, and the trap holds `&mut dyn Bus`.
    fn semihost_write(&mut self, _bytes: &[u8]) {}

    /// Pop up to `dst.len()` host semihosting-input bytes. Does not block.
    /// Default 0 (nothing available).
    fn semihost_read(&mut self, _dst: &mut [u8]) -> usize {
        0
    }

    /// A `bkpt #0xAB` has retired. Default no-op.
    fn semihost_note_attached(&mut self) {}

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

    /// Coalesce repeated reads of a peripheral register whose model promises
    /// a stable value through the next scheduled event. Default declines.
    fn repeat_stable_read_u32(&self, _addr: u64, _count: u32) -> Option<SimResult<u32>> {
        None
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
    /// A machine-armed stop for the current CPU batch (see
    /// `bus::BatchStop`). `step_batch` reads [`Self::batch_stop_armed`] once
    /// and then, while armed, ends the batch after the instruction for which
    /// [`Self::batch_stop_hit`] turns true. Default: never armed.
    fn batch_stop_armed(&self) -> bool {
        false
    }

    fn batch_stop_hit(&self) -> bool {
        false
    }

    /// A read-only view of plain memory (RAM or flash) holding `addr`, as
    /// `(start, end, bytes)`: reading it has no side effects and changes
    /// nothing. Used by poll-loop proofs. Default: [`Self::fetch_slice`].
    fn plain_memory_slice(&self, addr: u64) -> Option<(u64, u64, &[u8])> {
        self.fetch_slice(addr)
    }

    fn fetch_slice(&self, _pc: u64) -> Option<(u64, u64, &[u8])> {
        None
    }
}

use std::collections::HashSet;

/// Trait for controlling the machine in debug mode
pub trait DebugControl {
    /// The CPU's recorded fault, for the fault verdict. See
    /// [`Cpu::fault_capture`].
    fn fault_capture(&self) -> Option<fault_verdict::FaultCapture> {
        None
    }
    fn add_breakpoint(&mut self, addr: u32);
    fn remove_breakpoint(&mut self, addr: u32);
    fn clear_breakpoints(&mut self);

    /// Arm a debug watch range `[base, base + len)` reporting on the access
    /// kinds selected; false when the range is empty, wider than 4 KiB, or the
    /// watch cap is reached. The bus-side contract — what latches, when, and
    /// which access widths are watched — is the module `crates/core/src/bus/debug_watch.rs`.
    fn add_watchpoint(&mut self, _base: u64, _len: u64, _on_read: bool, _on_store: bool) -> bool {
        false
    }

    /// Disarm the exact watch [`Self::add_watchpoint`] armed; false when
    /// absent. See `crates/core/src/bus/debug_watch.rs` for the contract.
    fn remove_watchpoint(
        &mut self,
        _base: u64,
        _len: u64,
        _on_read: bool,
        _on_store: bool,
    ) -> bool {
        false
    }

    /// Drop every watch range and any hit that has not been reported yet.
    /// See `crates/core/src/bus/debug_watch.rs` for the contract.
    fn clear_watchpoints(&mut self) {}

    /// Run until breakpoint, watchpoint, or steps limit
    fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason>;

    /// Step a single instruction
    fn step_single(&mut self) -> SimResult<StopReason>;

    fn read_core_reg(&self, id: u8) -> u32;
    fn write_core_reg(&mut self, id: u8, val: u32);

    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>>;
    fn write_memory(&mut self, addr: u32, data: &[u8]) -> SimResult<()>;

    /// Side-effect-free decode of peripheral state, for debugger and agent views.
    ///
    /// Prefer this over [`Self::read_memory`] for anything a *human is merely
    /// looking at*. `read_memory` goes through the real bus read path, so it
    /// fires read side effects — a read-to-clear status register is cleared by
    /// the act of displaying it, and the firmware under test then misses the
    /// event. `inspect` uses `peek` throughout and cannot perturb the run.
    fn inspect(
        &self,
        name: Option<&str>,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::MachineInspect;

    /// Side-effect-free raw read. Bytes outside any mapped region or peripheral
    /// window come back as [`crate::inspect::PeekByte::Unmapped`] rather than a
    /// silent zero, so a debugger can render unmodeled space honestly instead of
    /// showing a convincing `0x00000000`.
    fn peek(&self, addr: u64, len: usize) -> crate::inspect::PeekResult;

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
    /// A debug watch range matched a CPU load/store at this address.
    Watchpoint(u64),
    StepDone,
    MaxStepsReached,
    ManualStop,
    /// The firmware ended its own run through the `simctl` device. Distinct
    /// from [`Self::ManualStop`], which is a *host* decision: this one carries
    /// the firmware's own exit code. See [`crate::peripherals::simctl`].
    FirmwareExit(u32),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// Counters collected during one measured machine run.
///
/// These are execution-path counters, not wall-clock or workload markers:
/// `cpu_instructions` counts retired instructions, `cpu_batches` counts CPU
/// dispatch batches, and the remaining fields describe peripheral/bus work
/// driven by that same pass. Workload-specific observables (for example an
/// OLED first-paint cycle or a serial completion marker) belong to the
/// workload harness and must not be added here.
pub struct StepProfile {
    pub cpu_instructions: u64,
    pub cpu_batches: u64,
    pub peripheral_ticks: u64,
    pub peripheral_ticked_entries: u64,
    pub bus_tick_entries: u64,
    pub legacy_tick_entries: u64,
}

/// Maximum cycles CPU idle fast-forward may skip in one step while a bus
/// peripheral needs bounded external-medium polling (a medium-mode WiFi MAC —
/// see [`Peripheral::idle_poll_bus_tick`]). Each skip runs the medium pump at
/// its deadline, so this sets the worst-case frame-delivery latency: ~8192
/// cycles ≈ 51 µs at 160 MHz, far under any WiFi association/DHCP/socket
/// timeout, while still collapsing the millions of idle cycles the CPU would
/// otherwise execute one-by-one. Non-WiFi buses never consult this. Only the
/// event-scheduler fast-forward path consults the medium poll quantum.
#[cfg(feature = "event-scheduler")]
const WIFI_MEDIUM_IDLE_POLL_QUANTUM: u64 = 8192;

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
    /// Stack pointer to give the secondary CPU when it is released, if the
    /// platform has one to give (ESP32 resolves `port_IntStackTop`). Set by
    /// whoever configures the platform; consumed in
    /// `release_secondary_cpu_if_requested` so a core is never released
    /// without a stack.
    pub secondary_boot_sp: Option<u32>,
    /// The secondary CPU has no ROM to boot: release it only on an explicit
    /// entry point (`APPCPU_BOOT_ADDR`), never on the reset-release edge.
    ///
    /// A faithful ROM boot constructs core 1 sitting on its reset vector and
    /// lets `SYSTEM_CORE_1_CONTROL_0.RESETING` 1->0 start it, exactly like
    /// silicon. The fast-boot frontends do not: they replace the mask ROM with
    /// a thunk harness, so that same vector holds no startup code and a core
    /// released there executes the harness and collapses to PC 0. Those
    /// frontends set this, and hand core 1 over at `call_start_cpu1` instead.
    pub secondary_awaits_boot_addr: bool,

    // Debug state
    pub breakpoints: std::collections::HashSet<u32>,
    pub last_breakpoint: Option<u32>,
    /// `last_breakpoint` for `cpu_secondary`: a dual-core run stops when
    /// either core reaches a breakpoint (an Arduino-ESP32 sketch's `loop()`
    /// runs on core 1).
    pub last_breakpoint_secondary: Option<u32>,
    /// The core the last breakpoint stop was on: 0 = `cpu`, 1 =
    /// `cpu_secondary`. Debuggers show and step this core.
    pub breakpoint_core: u8,
    pub total_cycles: u64,
    /// Cumulative CPU cycles advanced by idle fast-forward (WFI skip), not
    /// interpreted. Lets the browser `?perf=1` HUD prove FF is firing; 0 means
    /// either FF is off or firmware never parks in a skippable idle.
    pub idle_fast_forward_cycles_skipped: u64,
    /// The part of [`Self::idle_fast_forward_cycles_skipped`] skipped with at
    /// least one core in a proven cycle-counter poll loop
    /// ([`Cpu::poll_loop_skip_budget`]) rather than in WAITI/WFI.
    pub poll_loop_cycles_skipped: u64,
    /// Windows that made up [`Self::poll_loop_cycles_skipped`].
    pub poll_loop_skips: u64,
    /// Secondary-core cycles not interpreted because the core sat in a
    /// proven poll loop while the primary ran ([`Self::secondary_defer`]).
    pub poll_loop_cycles_deferred: u64,
    /// Primary batches run beside a deferred secondary instead of one
    /// instruction per tick (`Machine::run_primary_beside_quiet_secondary`).
    pub primary_batches_beside_deferred: u64,
    /// The secondary core's proven poll loop runs deferred: `(start, end)`
    /// machine cycles. The core is not stepped from `start`; it holds the
    /// state of its loop anchor at `start` and the plan its
    /// [`Cpu::poll_loop_skip_budget`] proved. The primary runs alone, as
    /// beside a parked core. The deferral ends at `end` (before the core's
    /// CCOMPARE0 edge), when [`Cpu::poll_loop_defer_changed`] reports a
    /// change (a word the loop reads, or a deliverable interrupt), and at
    /// the end of every [`Machine::advance`]; then
    /// [`Cpu::poll_loop_catch_up`] leaves the core where lockstep would.
    pub(crate) secondary_defer: Option<(u64, u64)>,
    /// Retained buffer for [`Cpu::poll_loop_watch`].
    batch_watch_scratch: Vec<(u64, u64)>,
    /// Plan CPU windows at `config.peripheral_tick_interval` (wide) while
    /// keeping every boundary cycle-exact: each batch ends right after an
    /// instruction that touches a device (`bus::BatchStop`), windows already
    /// end at scheduler and interrupt deadlines, and every boundary ticks. So the tick, IRQ aggregation and
    /// drain run on each cycle where interval 1 would see a change, and the
    /// cycles in between, where it would see none, run as one batch.
    ///
    /// Pair it with `bus.config.peripheral_tick_interval = 1`: models such as
    /// the UART pace their own events by the bus interval and quantise to it.
    /// Off by default (the browser runs wide and quantised); `labwired test`
    /// sets it for Cortex-M, where hosted runs must be cycle-exact.
    pub exact_batch_boundaries: bool,
    pub config: SimulationConfig,
    /// Injectable host clock used by [`HostTimeMode::Realtime`] pacing.
    host_clock: Box<dyn host_time::HostClock + Send>,
    step_profile: StepProfile,

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
    /// Cached bus index of the `simctl` device, when the bus declares one.
    /// Resolved once at construction so the advance loop's per-boundary drain
    /// is a single `Option` test on every board that does **not** use it —
    /// which is every board today. See [`crate::peripherals::simctl`].
    simctl_index: Option<usize>,
    /// Cached bus index of the SCB peripheral (Cortex-M). Resolved once at
    /// construction; `step()` drains a pending SYSRESETREQ latch every cycle
    /// and, when set, reboots the CPU through the vector table via the
    /// existing `Machine::reset`. `None` for non-Cortex-M targets (no SCB on
    /// the bus), so the per-cycle drain short-circuits without a peripheral
    /// walk or downcast.
    scb_index: Option<usize>,
    /// Cached bus index of the nRF52 NVMC peripheral. Resolved once at
    /// construction; the advance boundary drains a latched erase op every
    /// instruction and used to walk the whole ~40-entry peripheral list,
    /// paying a vtable call plus a `TypeId` compare per entry — on every chip,
    /// including the ones where an nRF52 NVMC cannot exist. `None` for every
    /// non-nRF52 target, so the drain short-circuits on one `Option` test.
    /// Same fixed-set assumption as [`Self::scb_index`]: nothing removes or
    /// reorders `bus.peripherals` after construction (appends are safe — they
    /// never move an existing index).
    nvmc_index: Option<usize>,
    /// Bus indices of peripherals whose `reset_request_capable()` is true,
    /// resolved once at construction. Empty on every chip that has no such
    /// peripheral, so the boundary drain is a single `is_empty()` test.
    /// Same fixed-set assumption as [`Self::scb_index`].
    reset_request_indices: Vec<usize>,
    /// A SoftDevice emulated at API level (nrf-softdevice-hle), when the
    /// image is an application built for one. `None` on every other board:
    /// the advance boundary then pays one `Option` test.
    pub sd_hle: Option<Box<sd_hle::SdHleSlot>>,
    /// SCB.VTOR as `load_firmware` left it (0, or the flash base / relocated
    /// table it retargets to). A SYSRESETREQ restores this value: VTOR is
    /// reset by a system reset, so a table the firmware moved to SRAM must not
    /// survive into the next boot. `None` until firmware is loaded (and on
    /// non-Cortex-M machines), in which case VTOR is left alone.
    boot_vtor: Option<u32>,
    /// Phase 2B.3b (issue #192): whether the one-time scheduler bootstrap has
    /// run. On the first `drain_scheduler_events`, peripherals with setup-time
    /// work (e.g. a UART with an RX stream attached before any MMIO write) get
    /// one chance to schedule their initial events. Always present; only read
    /// under the `event-scheduler` feature.
    #[allow(dead_code)]
    scheduler_bootstrapped: bool,
    /// Reusable scratch for the per-tick `tick_peripherals_fully` interrupt
    /// harvest, so the steady-state peripheral tick pushes pending NVIC IRQs
    /// into a retained buffer instead of allocating a fresh `Vec` every tick
    /// (the ~731k `RawVec::grow_one` the callgrind profile blamed on the C3
    /// SYSTIMER tick). Cleared, not reallocated, each tick.
    tick_irq_scratch: Vec<u32>,
    /// Reusable scratch for the per-tick peripheral-cost list, paired with
    /// [`Self::tick_irq_scratch`]. Empty on the walk-free fast path.
    tick_cost_scratch: Vec<bus::PeripheralTickCost>,
    /// Reusable scratch for `drain_scheduler_events`'s pending-schedule harvest.
    /// Swapped with `bus.pending_schedule` and drained in place so the drain
    /// reuses the buffer's capacity instead of `mem::take` freeing and later
    /// reallocating a fresh `Vec` every time write-context events were buffered.
    /// Semantics are identical (same entries, same order). Always present; only
    /// used under the `event-scheduler` feature.
    #[allow(dead_code)]
    pending_schedule_scratch: Vec<(usize, u64, u32)>,
    /// Reusable scratch for the due-events batch drained out of the scheduler
    /// each `drain_scheduler_events`. Taken out, filled by
    /// `EventScheduler::drain_due_into`, iterated, then restored — so the
    /// steady-state SYSTIMER tick (which drains an event nearly every batch)
    /// reuses the buffer's capacity instead of allocating a fresh `Vec` per
    /// drain. Always present; only used under the `event-scheduler` feature.
    #[allow(dead_code)]
    due_events_scratch: Vec<sched::ScheduledEvent>,
    /// Reusable no-op stand-in for the peripheral swap-out dance in
    /// `drain_scheduler_events` (a peripheral's `on_event` needs `&mut self.bus`,
    /// so the peripheral is temporarily replaced by a stub). Held here and
    /// swapped in/out so the hot scheduler event path (the ESP32-C3 SYSTIMER
    /// alarm fires one per drain) does not `Box::new` + free a fresh stub every
    /// event. Always `Some` between events. Only used under `event-scheduler`.
    #[allow(dead_code)]
    event_placeholder: Option<Box<dyn Peripheral>>,

    /// In-engine logic-analyzer edge capture (see [`crate::logic_capture`]).
    /// Empty/inactive by default — the step loop pays a single `is_active`
    /// check per step and nothing more until `logic_watch` installs a watch
    /// set. Not part of snapshot/restore: capture is a UI observation stream,
    /// re-armed by the frontend after a resume.
    logic_capture: logic_capture::LogicCapture,
    /// The analog waveform ring of the co-simulation runner driving this
    /// machine, when one is attached (see
    /// [`Machine::attach_analog_trace`]). `None` on every run with no analog
    /// co-sim model, and then [`Machine::analog_trace_snapshot`] answers an
    /// empty batch — an oscilloscope draws nothing rather than a flat line
    /// nothing measured.
    analog_trace: Option<crate::analog::AnalogTraceRegistry>,
    /// Test-only forcing knob (see [`Machine::logic_force_poll_capture`]):
    /// when `true`, `logic_watch` keeps every channel on the per-cycle poll
    /// path even for push-instrumented peripherals. This is what the
    /// differential oracle tests use to compare the two capture modes; it is
    /// NOT user-facing configuration.
    logic_force_poll: bool,
    /// What the WIRE half of the last watch set registered on each
    /// peripheral's [`PadLines`](crate::peripherals::pad_lines::PadLines), as
    /// `(peripheral index, channels per line)`.
    ///
    /// Remembered because a wire cell can also hold a PAD route's channel
    /// registrations — the same USART watched on PA2 and on its own `TX` line
    /// at once — and `PadLines::install_tap` would erase them. The arm path
    /// therefore MERGES, which needs to know what it itself put there last
    /// time so it can take exactly that back and nothing else. Same hazard
    /// [`PadRoutes::sync_taps`](crate::peripherals::pad_routing::PadRoutes::sync_taps)
    /// documents from the pad side, same fix.
    logic_wire_taps: Vec<(usize, Vec<Vec<u32>>)>,
    /// Four-state value of each watched channel at arm time (`None` where the
    /// pad's model reports no drive). Kept for the `result.json` series.
    logic_initial_states: Vec<Option<logic_capture::PadState>>,
    /// Shares the logic rings between a world's own pad watches (markers, GPIO
    /// nets) and an instrument's watch set. See `machine/world_hooks.rs`.
    observer: machine::world_hooks::ObserverMux,

    /// Cached bus index of the chip's authoritative simulated-µs source (first
    /// peripheral whose [`Peripheral::sim_time_us`] answers `Some` — the ESP32
    /// SYSTIMER). `None` on families with no absolute-µs counter (Cortex-M
    /// SysTick/TIM, nRF52, RP2040, SAMD); those now fall back to a clock DERIVED
    /// from `total_cycles` and [`crate::bus::SystemBus::cpu_hz`] rather than
    /// leaving every timed device always-ready — see
    /// [`Self::advance_central_i2c_time`]. Resolved once at construction, like
    /// [`Self::rtc_cntl_index`].
    i2c_time_source_index: Option<usize>,
    /// Cached bus indices of the controllers that opt into the central device
    /// time drive ([`Peripheral::drives_central_device_time`]) — I²C *and* SPI.
    /// Excludes the nRF54L TWIM, which drives its slaves' `advance_time_us`
    /// itself off the GRTC — so time is advanced exactly once. Empty ⇒ the
    /// drive short-circuits.
    i2c_time_controller_indices: Vec<usize>,
    /// Last authoritative µs the attached devices were advanced to. `u64::MAX`
    /// seeds "not yet anchored" so the first drive sets the mark without
    /// advancing (mirrors the nRF54L TWIM `last_us` seeding). A backward jump
    /// (SYSTIMER LOAD) re-anchors rather than advancing by a negative delta.
    last_i2c_time_us: u64,
    /// Has this machine already told the fidelity census that its device time
    /// is DERIVED from `cpu_hz` rather than read off a modelled µs counter?
    ///
    /// The census record is an honesty note, not a per-hit counter: it is worth
    /// saying once per run and nothing is learned by saying it a million times,
    /// so this latch gates it. Set the first time the derived clock actually
    /// hands a non-zero delta to a controller — a machine whose devices are
    /// never advanced makes no approximation and files no note.
    derived_device_time_noted: bool,
    /// Whether [`crate::fidelity::record_unpowered_rail_assumed`] has run for
    /// this machine: once, on the first advance with no routed supply.
    unpowered_rail_noted: bool,
    /// While a world runs this node until its next net pad drive change
    /// ([`Machine::advance_to_cycle_or_net_drive_change`]): the four-state
    /// ring's sequence number at the start, so the advance loop stops once
    /// it moves. `u64::MAX` otherwise.
    pub(crate) net_drive_stop_from: u64,
    /// Set by [`Machine::logic_observe`] once the ring moved past
    /// `net_drive_stop_from`; the advance loop stops on it.
    pub(crate) net_drive_stop_hit: bool,
}

impl<C: Cpu> Machine<C> {
    /// Register a [`SimulationObserver`] for the **whole** simulation.
    ///
    /// Observer events have two emitters and each keeps its own list:
    /// [`Machine`] publishes the CPU- and tick-shaped events
    /// (`on_step_start` / `on_step_end` / `on_peripheral_tick`), while
    /// [`bus::SystemBus`] publishes `on_memory_write`. The split is a
    /// borrow-checker fact — the CPU steps `&mut bus` while holding
    /// `&machine.observers` — not two audiences, and the two emitters never
    /// publish the same event, so an observer in both lists sees each event
    /// exactly once.
    ///
    /// Pushing straight to `machine.observers` therefore registers for half
    /// the events: that is why a `--vcd` trace recorded `pc` and nothing
    /// else, and why `--trace` carried no memory writes. Registering here
    /// instead of at either field is what makes an observer whole. An
    /// observer that genuinely wants only bus traffic (the DAP memory
    /// tracker, which outlives no `Machine`) still registers on the bus
    /// directly via [`bus::SystemBus::add_observer`].
    pub fn add_observer(&mut self, observer: Arc<dyn SimulationObserver>) {
        self.observers.push(observer.clone());
        self.bus.add_observer(observer);
    }

    /// Whether any logic-analyzer / signal probe is armed (poll or push mode).
    /// The `jit_framework` [`SafetyGate`](crate::cpu::jit_framework::fallback::SafetyGate)
    /// reads this to force the interpreter while a probe needs per-cycle pad
    /// visibility. `logic_capture` is module-private, so this crate-internal
    /// accessor is how the RISC-V JIT host reaches it.
    #[cfg(any(feature = "jit", feature = "jit-framework"))]
    pub(crate) fn logic_probes_active(&self) -> bool {
        self.logic_capture.poll_active() || self.logic_capture.push_active()
    }

    /// Discover the drivable input channels on this machine (delegates to
    /// [`bus::SystemBus::list_inputs`]). See [`crate::sim_input`].
    pub fn list_inputs(&mut self) -> Vec<(String, crate::sim_input::InputChannel)> {
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
        self.bus.set_input(None, channel, value)
    }

    /// [`Machine::set_input`] narrowed to the device named `component` — the
    /// external-device id from system.yaml (stamped onto the model at attach)
    /// or the owning peripheral's bus name; the disambiguator a test-script
    /// stimulus `target.component` resolves through when two devices expose
    /// the same channel key.
    pub fn set_input_on(
        &mut self,
        component: &str,
        channel: &str,
        value: f64,
    ) -> Result<(), crate::sim_input::SimInputError> {
        self.bus.set_input(Some(component), channel, value)
    }

    /// Apply several input sets as one atomic transaction (delegates to
    /// [`bus::SystemBus::set_inputs`]): every set is validated first and
    /// either all apply or none do, with no execution in between — the way to
    /// drive a multi-channel pose (an IMU's x/y/z, a GPS lat+lon) without the
    /// firmware ever observing a torn update.
    pub fn set_inputs(
        &mut self,
        sets: &[(Option<&str>, &str, f64)],
    ) -> Result<(), crate::sim_input::SimInputError> {
        self.bus.set_inputs(sets)
    }

    /// Install a logic-analyzer watch set, resetting the capture buffer and
    /// cursor. `resolved[i]` is `Some(source)` for a resolvable ref — a
    /// [`LogicSource::Pad`](logic_capture::LogicSource::Pad) or a
    /// [`LogicSource::Wire`](logic_capture::LogicSource::Wire) — or `None` for
    /// an unresolvable one (never sampled). Returns each channel's initial
    /// level (`None` = unknown), same order as `resolved`, so the caller can
    /// seed the waveform before the first edge. Passing an empty slice disarms
    /// capture.
    ///
    /// Each resolvable channel is armed in one of two modes: push
    /// (event-driven — for a pad, the owning peripheral accepted
    /// [`Peripheral::install_logic_tap`] and reports pad writes itself; for a
    /// wire, this method registers the channel on the peripheral's own
    /// [`PadLines`](crate::peripherals::pad_lines::PadLines)) or the per-cycle
    /// poll fallback. See [`crate::logic_capture`].
    ///
    /// Push is not an optimisation for a wire channel, it is the only correct
    /// mode: a transaction-level model narrates a whole finished burst through
    /// [`PadLines::set_line_at`](crate::peripherals::pad_lines::PadLines::set_line_at)
    /// in one call, so a per-cycle poller would see only the LAST level of the
    /// burst and every intermediate edge would be lost.
    pub fn logic_watch(
        &mut self,
        resolved: &[Option<logic_capture::LogicSource>],
    ) -> Vec<Option<bool>> {
        use logic_capture::LogicSource;

        // Group the PAD half of the watch set (probe and own-drive channels)
        // per owning peripheral. Wire channels are handled below, and are
        // deliberately not offered to the pad watch: that hook is about pads,
        // and a peripheral answering it must not have to guess which of its
        // own lines a pin number meant.
        let mut per_peripheral: std::collections::HashMap<usize, Vec<pins::WatchedPad>> =
            std::collections::HashMap::new();
        if !self.logic_force_poll {
            for (ch, r) in resolved.iter().enumerate() {
                let (peripheral, pin, view) = match *r {
                    Some(LogicSource::Pad { peripheral, pin }) => {
                        (peripheral, pin, pins::PadView::Probe)
                    }
                    Some(LogicSource::Driver { peripheral, pin }) => {
                        (peripheral, pin, pins::PadView::Driver)
                    }
                    _ => continue,
                };
                per_peripheral
                    .entry(peripheral)
                    .or_default()
                    .push(pins::WatchedPad {
                        pin,
                        ch: ch as u32,
                        view,
                    });
            }
        }

        // Offer every peripheral its slice of the watch set (empty ⇒ clears a
        // previously installed watch). Whether a peripheral ACCEPTS is its own
        // declaration of push capability — no hardcoded list here. A pin port
        // takes a generic `PadWatch`; anything else that owns pads (an RMT
        // bit engine) answers the older `install_logic_tap`.
        let tap = self.bus.logic_tap.clone();
        let mut push = vec![false; resolved.len()];
        static EMPTY: [pins::WatchedPad; 0] = [];
        for (idx, p) in self.bus.peripherals.iter_mut().enumerate() {
            let watched: &[pins::WatchedPad] =
                per_peripheral.get(&idx).map_or(&EMPTY, |v| v.as_slice());
            let accepted = match p.dev.pins_mut() {
                Some(port) => port.install_watch(pins::PadWatch::new(&tap, watched)),
                None => {
                    let pairs: Vec<(u8, u32)> = watched
                        .iter()
                        .filter(|w| w.view == pins::PadView::Probe)
                        .map(|w| (w.pin, w.ch))
                        .collect();
                    p.dev.install_logic_tap(&tap, &pairs) && pairs.len() == watched.len()
                }
            };
            if accepted {
                for w in watched {
                    push[w.ch as usize] = true;
                }
            }
        }

        // ── The WIRE half ───────────────────────────────────────────────────
        // AFTER the pad pass, never before. A GPIO port with no watched pin
        // clears the pad-route taps on the very cells wire channels register
        // on (`PadRoutes::clear_taps`), and ports are visited in bus-index
        // order — registering first would simply be erased by a later port.
        let mut wire_now: std::collections::HashMap<usize, Vec<Vec<u32>>> =
            std::collections::HashMap::new();
        if !self.logic_force_poll {
            for (ch, r) in resolved.iter().enumerate() {
                let Some(LogicSource::Wire { peripheral, line }) = *r else {
                    continue;
                };
                let Some(width) = self
                    .bus
                    .peripherals
                    .get(peripheral)
                    .and_then(|p| p.dev.wire_lines())
                    .map(|lines| lines.names().len())
                else {
                    continue;
                };
                let slot = wire_now
                    .entry(peripheral)
                    .or_insert_with(|| vec![Vec::new(); width]);
                if let Some(channels) = slot.get_mut(line) {
                    channels.push(ch as u32);
                    push[ch] = true;
                }
            }
        }
        // Take back exactly what the previous watch set registered here and
        // install what this one wants — including on peripherals that dropped
        // out of the watch set entirely, which is what disarms them.
        let stale = std::mem::take(&mut self.logic_wire_taps);
        let mut touched: Vec<usize> = wire_now.keys().copied().collect();
        for (idx, _) in &stale {
            if !touched.contains(idx) {
                touched.push(*idx);
            }
        }
        const NO_CHANNELS: &[Vec<u32>] = &[];
        for idx in touched {
            let remove = stale
                .iter()
                .find(|(i, _)| *i == idx)
                .map_or(NO_CHANNELS, |(_, channels)| channels.as_slice());
            let add = wire_now.get(&idx).map_or(NO_CHANNELS, Vec::as_slice);
            if let Some(lines) = self
                .bus
                .peripherals
                .get(idx)
                .and_then(|p| p.dev.wire_lines())
            {
                lines.merge_tap(Some(tap.clone()), remove, add);
            }
        }
        self.logic_wire_taps = wire_now
            .into_iter()
            .filter(|(_, channels)| channels.iter().any(|line| !line.is_empty()))
            .collect();

        let bus = &self.bus;
        let initial: Vec<Option<bool>> = resolved
            .iter()
            .map(|r| r.and_then(|source| Self::read_logic_source(bus, source)))
            .collect();
        self.logic_capture.install(resolved, &initial, &push);
        let drives: Vec<Option<logic_capture::PadDrive>> = resolved
            .iter()
            .map(|r| r.and_then(|source| Self::read_logic_drive(bus, source)))
            .collect();
        self.logic_initial_states = self.logic_capture.install_drives(&drives);

        // Arm the tap clock at "the next observation boundary" so pushes that
        // happen before any stepping (e.g. a paused-machine input change)
        // stamp where the first post-watch sample would observe them.
        tap.clear_events();
        tap.set_clock(self.total_cycles + 1);
        tap.set_armed(self.logic_capture.push_active());
        initial
    }

    /// Test-only forcing knob for the differential capture oracle: when
    /// `true`, subsequent [`logic_watch`](Self::logic_watch) calls keep every
    /// channel on the per-cycle poll path even where push instrumentation
    /// exists (the batch clamp and idle-fast-forward disable apply again).
    /// Takes effect at the next `logic_watch`. Not user-facing configuration —
    /// it exists so tests can assert push and poll produce byte-identical
    /// edge streams.
    #[doc(hidden)]
    pub fn logic_force_poll_capture(&mut self, force: bool) {
        self.logic_force_poll = force;
    }

    /// Read logic edges newer than `cursor`, acknowledging retained edges
    /// before it (see [`logic_capture::LogicCapture::read_edges`]).
    pub fn logic_read_edges(&mut self, cursor: u64) -> logic_capture::LogicEdgeBatch {
        self.logic_capture.read_edges(cursor)
    }

    /// Read four-state (`0`/`1`/`z`/`x`) transitions newer than `cursor`, on
    /// the state ring's own cursor space. Only channels whose pad model
    /// reports drive produce any.
    pub fn logic_read_states(&mut self, cursor: u64) -> logic_capture::LogicStateBatch {
        self.logic_capture.read_states(cursor)
    }

    /// Four-state value of each watched channel at arm time, by channel.
    pub fn logic_initial_states(&self) -> &[Option<logic_capture::PadState>] {
        &self.logic_initial_states
    }

    /// Publish a co-simulation runner's analog waveform ring on this machine,
    /// so instruments can read it without knowing where the runner lives.
    ///
    /// The adapter that fills the ring sits inside
    /// [`CosimRunner`](crate::cosim::CosimRunner), which the machine does not
    /// own; the registry is a cheap shared handle, so whoever holds the runner
    /// registers it here once and the two stay in step.
    pub fn attach_analog_trace(&mut self, registry: crate::analog::AnalogTraceRegistry) {
        self.analog_trace = Some(registry);
    }

    /// Whether a co-simulation runner's analog trace is attached. Separates
    /// "nothing is driving an analog model" from "a runner is attached but none
    /// of its models records a waveform", which an empty channel table alone
    /// cannot.
    pub fn analog_trace_attached(&self) -> bool {
        self.analog_trace.is_some()
    }

    /// Channel table of the attached analog trace; empty when none is.
    pub fn analog_channels(&self) -> Vec<crate::analog::AnalogChannel> {
        self.analog_trace
            .as_ref()
            .map(|registry| registry.channels())
            .unwrap_or_default()
    }

    /// Analog samples newer than `cursor` (a sample sequence number, the same
    /// cursor contract as [`Machine::logic_read_edges`]). An empty batch when
    /// no co-simulation runner is attached.
    pub fn analog_trace_snapshot(&self, cursor: u64) -> crate::analog::AnalogTraceBatch {
        self.analog_trace
            .as_ref()
            .map(|registry| registry.snapshot(cursor))
            .unwrap_or_default()
    }

    /// Run-long change-point series of every analog channel (`result.json`'s
    /// `analog_series`); `None` when no co-simulation runner is attached.
    pub fn analog_series(&self) -> Option<crate::analog::AnalogSeries> {
        self.analog_trace.as_ref().map(|registry| registry.series())
    }

    /// Current engine cycle — the `nowCycle` reported alongside a logic-edge
    /// read so the UI can extend flat traces to "now".
    pub fn logic_now_cycle(&self) -> u64 {
        self.total_cycles
    }

    /// `true` while at least one watched channel is on the per-cycle poll
    /// fallback. Frontends inspect this when choosing an outer request batch
    /// limit; [`Machine::advance`] independently clamps each internal batch to
    /// one so polled pads are sampled at every cycle boundary. Push-only watch
    /// sets keep the full batch width because their peripherals report edges
    /// from the write sites.
    #[inline]
    pub fn logic_poll_active(&self) -> bool {
        self.logic_capture.poll_active()
    }

    /// Cumulative transition totals per watched channel since the watch set was
    /// installed (same `ch` indexing as [`Machine::logic_read_edges`]). Unlike
    /// a drained edge batch this is not a bounded window, so it is the honest
    /// "has this pad toggled N times" evidence a stop condition can consult
    /// mid-run (the CLI's `gpio_edges` assertion).
    #[inline]
    pub fn logic_channel_edge_counts(&self) -> &[u64] {
        self.logic_capture.channel_edge_counts()
    }

    /// Observe the watched channels at the current cycle boundary: drain the
    /// push tap (event-driven channels) and sample the polled channels.
    /// Hooked into the step loop; the leading `is_active` guard is the entire
    /// cost when nothing is watched.
    ///
    /// `boundary` is the engine cycle at the end of the just-executed
    /// instruction batch, BEFORE any peripheral tick-cost cycles were charged
    /// — pushes stamped at it are finalised to `total_cycles` ("now"), which
    /// is where a per-cycle poll would have seen them.
    #[inline]
    fn logic_observe(&mut self, boundary: u64) {
        if !self.logic_capture.is_active() {
            return;
        }
        let now = self.total_cycles;
        if self.logic_capture.push_active() {
            let mut events = if self.bus.logic_tap.pending_len() == 0 {
                Vec::new()
            } else {
                self.bus.logic_tap.take_events()
            };
            if !events.is_empty() {
                // `ingest_push` groups ADJACENT equal-cycle runs, so it needs
                // ascending stamps. A bit engine pushes in engine order and is
                // already sorted; a transaction-level controller narrating a
                // completed phase via `LogicTap::push_at` emits past-stamped
                // edges that can land after a live push from another
                // peripheral. Sort only when that actually happened — the
                // check is one pass over a queue that is empty on almost every
                // boundary, and a stable sort keeps same-cycle last-wins order
                // (so an already-sorted queue ingests byte-identically).
                if events.windows(2).any(|w| w[0].cycle > w[1].cycle) {
                    events.sort_by_key(|event| event.cycle);
                }
                self.logic_capture.ingest_push(&events, boundary, now);
            }
            // Re-arm the provisional stamp at the NEXT boundary so pad writes
            // arriving while the machine is paused (sim-input, button pushes)
            // stamp where the first post-resume observation would see them.
            self.bus.logic_tap.set_clock(now + 1);
        }
        if self.logic_capture.poll_active() {
            let bus = &self.bus;
            self.logic_capture.sample_with_drive(
                now,
                |source| Self::read_logic_source(bus, source),
                |source| Self::read_logic_drive(bus, source),
            );
        }
        // A world running this node until its next net pad drive change
        // (`advance_to_cycle_or_net_drive_change`) stops here.
        if self.logic_capture.state_seq() > self.net_drive_stop_from {
            self.net_drive_stop_hit = true;
        }
    }

    /// The drive one analyzer channel reads right now: a pad's model answers
    /// through [`Peripheral::read_gpio_pad_drive`] ([`pins::probe_drive`]), a
    /// chip's own output stage through [`pins::own_drive`]; a peripheral WIRE
    /// is by definition driven by that peripheral.
    fn read_logic_drive(
        bus: &bus::SystemBus,
        source: logic_capture::LogicSource,
    ) -> Option<logic_capture::PadDrive> {
        match source {
            logic_capture::LogicSource::Pad { peripheral, pin } => bus
                .peripherals
                .get(peripheral)
                .and_then(|p| p.dev.read_gpio_pad_drive(pin)),
            logic_capture::LogicSource::Driver { peripheral, pin } => bus
                .peripherals
                .get(peripheral)
                .and_then(|p| p.dev.pins())
                .and_then(|port| pins::own_drive(port, pin)),
            logic_capture::LogicSource::Wire { .. } => Some(logic_capture::PadDrive::Driven),
        }
    }

    /// The level one analyzer channel reads right now. The ONE place the two
    /// channel kinds are told apart, so arming (`logic_watch`'s initial
    /// levels) and sampling can never disagree about what a channel means.
    ///
    /// ⚠️ A `Pad` reads `read_gpio_pad` and stops there. There is deliberately
    /// no fallback to the peripheral's wire: an unmuxed pad IS the GPIO latch,
    /// and a probe that quietly showed bus traffic on a pin no bus reaches
    /// would be lying about the exact thing it was clipped on to answer.
    fn read_logic_source(bus: &bus::SystemBus, source: logic_capture::LogicSource) -> Option<bool> {
        match source {
            logic_capture::LogicSource::Pad { peripheral, pin } => bus
                .peripherals
                .get(peripheral)
                .and_then(|p| p.dev.read_gpio_pad(pin)),
            logic_capture::LogicSource::Driver { peripheral, pin } => bus
                .peripherals
                .get(peripheral)
                .and_then(|p| p.dev.pins())
                .and_then(|port| port.level(pin)),
            logic_capture::LogicSource::Wire { peripheral, line } => bus
                .peripherals
                .get(peripheral)
                .and_then(|p| p.dev.wire_lines())
                // Out of range reads UNKNOWN, not low: `PadLines::level`
                // answers `false` for a stale index so a pad read can never
                // panic the engine, and taking that at face value here would
                // draw a confident flat-low trace for a line that does not
                // exist.
                .filter(|lines| line < lines.names().len())
                .map(|lines| lines.level(line)),
        }
    }

    /// Resolve a wire reference — a peripheral NAME and a line NAME — to a
    /// [`LogicSource::Wire`](logic_capture::LogicSource::Wire).
    ///
    /// This is the door a frontend, a test or an agent comes through, and it
    /// is the reason a wire channel is addressed by name at all: `usart2.TX`
    /// survives a bus reorder, an AF-table edit and a chip-config rename,
    /// where a line index does not.
    ///
    /// Line names match ignoring case. Every failure is REPORTED rather than
    /// swallowed — see [`LogicRefError`](logic_capture::LogicRefError).
    pub fn resolve_wire_source(
        &self,
        peripheral: &str,
        line: &str,
    ) -> Result<logic_capture::LogicSource, logic_capture::LogicRefError> {
        let Some(idx) = self.bus.find_peripheral_index_by_name(peripheral) else {
            return Err(logic_capture::LogicRefError::UnknownPeripheral {
                peripheral: peripheral.to_string(),
            });
        };
        let names = self.bus.peripherals[idx].dev.line_names();
        if names.is_empty() {
            return Err(logic_capture::LogicRefError::NoWireLines {
                peripheral: peripheral.to_string(),
            });
        }
        match logic_capture::line_index_of(names, line) {
            Some(index) => Ok(logic_capture::LogicSource::wire(idx, index)),
            None => Err(logic_capture::LogicRefError::UnknownLine {
                peripheral: peripheral.to_string(),
                line: line.to_string(),
                available: names.to_vec(),
            }),
        }
    }

    /// The wire line names `peripheral` publishes, for a frontend listing what
    /// can be probed. Empty for a peripheral with no wire.
    pub fn wire_line_names(&self, peripheral: &str) -> &'static [&'static str] {
        self.bus
            .find_peripheral_index_by_name(peripheral)
            .map_or(&[], |idx| self.bus.peripherals[idx].dev.line_names())
    }

    /// Every peripheral on this bus that publishes wire lines, with the lines
    /// it publishes — the whole probeable wire surface, in one answer.
    ///
    /// A frontend that builds a probe menu MUST read it from here. The line
    /// vocabulary is not uniform and cannot be guessed from the protocol: the
    /// generic STM32 SPI publishes `SCK/MOSI/MISO` with no chip-select line at
    /// all, RP2040 SPI spells its select `CSn`, and the ESP GPSPI spells it
    /// `CS`. A UI that hardcodes one spelling offers lanes that cannot resolve
    /// on two families out of three.
    pub fn wire_surface(&self) -> Vec<(&str, &'static [&'static str])> {
        self.bus
            .peripherals
            .iter()
            .filter_map(|p| {
                let names = p.dev.line_names();
                (!names.is_empty()).then_some((p.name.as_str(), names))
            })
            .collect()
    }

    /// The level a already-resolved channel source reads right now, without
    /// arming capture. Same [`read_logic_source`](Self::read_logic_source)
    /// body the watch path uses, so a live readout and a captured waveform can
    /// never disagree about what a channel means.
    pub fn read_logic_level(&self, source: logic_capture::LogicSource) -> Option<bool> {
        Self::read_logic_source(&self.bus, source)
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
        let simctl_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::simctl::SimCtl>())
                .is_some()
        });
        let scb_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::scb::Scb>())
                .is_some()
        });
        let reset_request_indices: Vec<usize> = bus
            .peripherals
            .iter()
            .enumerate()
            .filter(|(_, p)| p.dev.reset_request_capable())
            .map(|(i, _)| i)
            .collect();
        let nvmc_index = bus.peripherals.iter().position(|p| {
            p.dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::nrf52::nvmc::Nrf52Nvmc>())
                .is_some()
        });
        // Central I²C data-ready time drive (Option A): the authoritative µs
        // source is the first peripheral that reports one (ESP32 SYSTIMER); the
        // fan-out targets are the opted-in I²C controllers. Both are stable for
        // the life of the bus (chip peripherals are added at assembly; slaves
        // attach to already-present controllers), so resolve once here.
        let i2c_time_source_index = bus
            .peripherals
            .iter()
            .position(|p| p.dev.sim_time_us().is_some());
        let i2c_time_controller_indices = bus
            .peripherals
            .iter()
            .enumerate()
            .filter(|(_, p)| p.dev.drives_central_device_time())
            .map(|(i, _)| i)
            .collect();
        // Silent-path census, counter (b2) — measurement only, and an empty
        // `#[inline(always)]` fn without the `silent-census` feature.
        //
        // This is the ONE place the census can honestly ask "what is still a
        // stub?": `Machine::new` is the single choke every runner (CLI lab
        // runner, environment runner, wasm, DAP, python, `system::node`) passes
        // through with a *finished* bus, so `from_config` and every
        // post-factory replacement pass — `configure_cortex_m`'s NVIC/SCB/DWT
        // install above all — have already run. Counting stubs at the factory
        // instead reports peripherals that were replaced by real models before
        // the first instruction; that error is what this counter exists to
        // correct. Identity is by `TypeId`, never by name.
        crate::census::record_live_stubs(&bus);
        Self {
            cpu,
            cpu_secondary: None,
            bus,
            observers: Vec::new(),
            secondary_boot_sp: None,
            secondary_awaits_boot_addr: false,
            breakpoints: HashSet::new(),
            last_breakpoint: None,
            last_breakpoint_secondary: None,
            breakpoint_core: 0,
            total_cycles: 0,
            idle_fast_forward_cycles_skipped: 0,
            poll_loop_cycles_skipped: 0,
            poll_loop_skips: 0,
            poll_loop_cycles_deferred: 0,
            primary_batches_beside_deferred: 0,
            secondary_defer: None,
            batch_watch_scratch: Vec::new(),
            exact_batch_boundaries: false,
            config: SimulationConfig::default(),
            host_clock: Box::new(host_time::StdHostClock::new()),
            step_profile: StepProfile::default(),
            sched: sched::EventScheduler::new(),
            clocks: sched::ClockGraph::new(),
            rtc_cntl_index,
            flash_index,
            simctl_index,
            scb_index,
            nvmc_index,
            reset_request_indices,
            sd_hle: None,
            boot_vtor: None,
            scheduler_bootstrapped: false,
            tick_irq_scratch: Vec::new(),
            tick_cost_scratch: Vec::new(),
            pending_schedule_scratch: Vec::new(),
            due_events_scratch: Vec::new(),
            event_placeholder: Some(Box::new(crate::peripherals::stub::StubPeripheral::new(0))),
            logic_capture: logic_capture::LogicCapture::new(),
            analog_trace: None,
            logic_force_poll: false,
            logic_wire_taps: Vec::new(),
            logic_initial_states: Vec::new(),
            observer: Default::default(),
            i2c_time_source_index,
            i2c_time_controller_indices,
            last_i2c_time_us: u64::MAX,
            derived_device_time_noted: false,
            unpowered_rail_noted: false,
            net_drive_stop_from: u64::MAX,
            net_drive_stop_hit: false,
        }
    }

    /// Advance every centrally-driven off-chip device's clock — I²C, SPI or
    /// UART peer — to the chip's simulated-µs "now". Called after each committed
    /// CPU boundary and each committed architectural idle fast-forward boundary.
    ///
    /// **Two sources, in strict preference order.**
    ///
    /// 1. A peripheral that models a genuine absolute-µs counter
    ///    ([`Peripheral::sim_time_us`] — the ESP32 SYSTIMER). An absolute
    ///    counter beats anything derived: it already accounts for its own
    ///    clock tree, so the ESP32 path is byte-identical to before.
    /// 2. Otherwise, DERIVED from the committed cycle count (including idle) and the system's
    ///    declared core clock: `cycles * 1_000_000 / cpu_hz`, integer, monotonic
    ///    (`total_cycles` only grows, `cpu_hz` is fixed for the life of the
    ///    bus). This is what makes a declarative `data_ready` / `delay_us`
    ///    device real on STM32, nRF52, RP2040 and SAMD, where it used to be a
    ///    silent thunk: always-ready, instantly, on every chip with no SYSTIMER.
    ///
    /// **The derived clock is an approximation and says so.** `cpu_hz` is the
    /// manifest/chip-declared core frequency; firmware that reconfigures the PLL
    /// mid-run moves the real core clock and this model does not follow it. That
    /// is recorded ONCE per run in the fidelity census
    /// ([`crate::fidelity::record_derived_device_time`]) so the browser and the
    /// CLI can show the reader what they are looking at, rather than the engine
    /// quietly presenting a derived number as a measured one.
    ///
    /// The nRF54L TWIM is never in the controller list (it does not opt in), so
    /// its slaves are advanced exactly once — by TWIM itself, off the GRTC.
    fn advance_central_i2c_time(&mut self) {
        if self.i2c_time_controller_indices.is_empty() {
            return;
        }
        let (now, derived) = match self.i2c_time_source_index {
            Some(src) => match self.bus.peripherals[src].dev.sim_time_us() {
                Some(now) => (now, false),
                None => return,
            },
            None => {
                let hz = self.bus.cpu_hz;
                if hz == 0 {
                    // No declared core clock ⇒ nothing honest to derive from.
                    return;
                }
                // u128 so a long run cannot wrap: `total_cycles * 1_000_000`
                // overflows u64 at ~1.8e13 cycles, which a 160 MHz part reaches
                // in about 32 hours of simulated time.
                let us = (u128::from(self.total_cycles) * 1_000_000u128) / u128::from(hz);
                (us as u64, true)
            }
        };
        let last = self.last_i2c_time_us;
        if last == u64::MAX || now < last {
            // First slice, or a counter reload jumped time backward: re-anchor
            // without advancing (never hand a slave a negative delta).
            self.last_i2c_time_us = now;
            return;
        }
        if now == last {
            return;
        }
        let delta = now - last;
        if derived && !self.derived_device_time_noted {
            // First time the DERIVED clock actually moves a device on this
            // machine. Said once — see the field's note.
            self.derived_device_time_noted = true;
            crate::fidelity::record_derived_device_time(self.bus.cpu_hz);
        }
        // Index by value so the immutable read of the cached list and the
        // mutable peripheral borrow don't overlap.
        for i in 0..self.i2c_time_controller_indices.len() {
            let idx = self.i2c_time_controller_indices[i];
            self.bus.peripherals[idx]
                .dev
                .advance_attached_device_time_us(delta);
        }
        self.last_i2c_time_us = now;
    }

    /// Enable dual-core mode by attaching a secondary CPU instance.
    /// The secondary CPU shares the same bus and steps in lockstep with
    /// the primary (one instruction each per `Machine::step()`). Used by
    /// ESP32 configs to model APP_CPU alongside PRO_CPU.
    pub fn with_secondary_cpu(mut self, cpu1: C) -> Self {
        self.cpu_secondary = Some(cpu1);
        self
    }

    /// Replace the host clock. Tests inject a fake that records sleeps.
    pub fn with_host_clock(mut self, clock: Box<dyn host_time::HostClock + Send>) -> Self {
        self.host_clock = clock;
        self
    }

    /// The largest `peripheral_tick_interval` THIS MACHINE can run at without
    /// losing fidelity. Wraps [`crate::bus::SystemBus::max_safe_tick_interval`]
    /// and adds the one clause the bus cannot see: **a machine with a second
    /// CPU must stay at 1.**
    ///
    /// The bus predicate answers "is every peripheral scheduler-driven", which
    /// bounds *peripheral* observation error to one interval. On a single-core
    /// machine that is the whole story. On an SMP machine it is not: the cores
    /// synchronise through interrupts, and an interrupt-delivery skew of up to
    /// one interval is not an observation error, it is a *semantic* one. ESP-IDF
    /// SMP FreeRTOS implements `portYIELD_WITHIN_API()` as
    /// `esp_crosscore_int_send_yield(xPortGetCoreID())` — ring your own core's
    /// doorbell inside a critical section and be preempted the instant
    /// `portEXIT_CRITICAL` re-enables interrupts. Land that late and the calling
    /// task falls out of `xQueueReceive`'s `for(;;)` still runnable, blocks a
    /// second time on an event list it is already on, and `vListInsert` links
    /// the item to itself: the next insert walks the self-loop forever holding
    /// the queue spinlock while the other core spins in `spinlock_acquire`.
    /// Measured on the ESP32-S3 dual-core Doom lab — deadlock at interval 512,
    /// `frame 1` at interval 1, same firmware, same engine, one variable. The
    /// unicore build of the same firmware runs fine at 512, which is exactly the
    /// line this clause draws.
    ///
    /// Frontends that batch (the browser's `recommended_tick_interval`) must ask
    /// the MACHINE, not the bus.
    pub fn max_safe_tick_interval(&self) -> u32 {
        if self.cpu_secondary.is_some() {
            return 1;
        }
        self.bus.max_safe_tick_interval()
    }

    pub fn reset_step_profile(&mut self) {
        self.step_profile = StepProfile::default();
    }

    pub fn step_profile(&self) -> StepProfile {
        self.step_profile
    }

    /// Wall-clock attribution for the open [`profile`] window, with peripheral
    /// indices resolved to their bus names.
    ///
    /// [`StepProfile`] counts events; this measures the time they cost. The two
    /// disagree sharply — on the BLE Pong lab `i2c0` owns 67 % of scheduler
    /// arms but 18 % of wall time — so rank optimisation work by this one.
    pub fn profile_report(&self) -> profile::Report {
        let names: Vec<String> = self
            .bus
            .peripherals
            .iter()
            .map(|p| p.name.clone())
            .collect();
        profile::snapshot().report(&names)
    }

    fn record_peripheral_tick_profile(&mut self, cost_entries: usize) {
        let (bus_tick_entries, legacy_tick_entries) = self.bus.tick_profile_entry_counts();
        self.step_profile.peripheral_ticks += 1;
        self.step_profile.peripheral_ticked_entries += cost_entries as u64;
        self.step_profile.bus_tick_entries += bus_tick_entries as u64;
        self.step_profile.legacy_tick_entries += legacy_tick_entries as u64;
    }

    /// The secondary core's state as the window planner and the CPU window
    /// see it: a deferred poll loop runs like a parked core.
    pub(crate) fn secondary_plan_state(&self) -> Option<SecondaryExecutionState> {
        if self.secondary_defer.is_some() {
            return Some(SecondaryExecutionState::ParkedIdle);
        }
        self.cpu_secondary
            .as_ref()
            .map(|sec| sec.secondary_execution_state())
    }

    /// End the secondary's deferral with the core caught up to `through`:
    /// its steps for the machine cycles before `through` ran on the state
    /// the proof saw.
    pub(crate) fn end_secondary_defer(&mut self, through: u64) {
        let Some((start, _)) = self.secondary_defer.take() else {
            return;
        };
        let cycles = through.saturating_sub(start);
        if let Some(sec) = self.cpu_secondary.as_mut() {
            let caught_up = sec.poll_loop_catch_up(&self.bus, cycles);
            debug_assert!(caught_up, "a deferred poll loop must catch up");
        }
        self.poll_loop_cycles_deferred += cycles;
    }

    /// At a machine boundary (after the tick and the scheduler drain): end a
    /// deferral that is due, with the core caught up to now.
    pub(crate) fn check_secondary_defer(&mut self) -> bool {
        let Some((_, end)) = self.secondary_defer else {
            return false;
        };
        let changed = self.total_cycles >= end
            || self
                .cpu_secondary
                .as_ref()
                .is_none_or(|sec| sec.poll_loop_defer_changed(&self.bus));
        if changed {
            self.end_secondary_defer(self.total_cycles);
        }
        changed
    }

    /// Defer the secondary core's proven poll loop so the primary can run
    /// alone (see [`Self::secondary_defer`]). Only on a quantum-1 bus, where
    /// the primary-alone window checks the deferral after every instruction.
    pub(crate) fn try_defer_secondary(&mut self, limit: u64) -> bool {
        if self.secondary_defer.is_some()
            || !self.config.idle_fast_forward_enabled
            || self.config.peripheral_tick_interval.max(1) != 1
            || self.logic_capture.poll_active()
            || !self.breakpoints.is_empty()
        {
            return false;
        }
        let Some(sec) = self.cpu_secondary.as_mut() else {
            return false;
        };
        if sec.secondary_execution_state() != SecondaryExecutionState::Active
            || !sec.poll_loop_anchor_ready()
        {
            return false;
        }
        let edge = sec.parked_wake_deadline_cycles();
        let Some((_, cycles)) = sec.poll_loop_skip_budget(&self.bus, limit) else {
            return false;
        };
        // The step that raises CCOMPARE0 (the `edge`-th from now) runs live.
        let len = edge.map_or(cycles, |edge| cycles.min(edge.saturating_sub(1)));
        if len < 2 {
            sec.poll_loop_catch_up(&self.bus, 0);
            return false;
        }
        self.secondary_defer = Some((self.total_cycles, self.total_cycles + len));
        true
    }

    fn try_idle_fast_forward(&mut self, _max_steps: Option<u64>, _steps: u64) -> u64 {
        // POLLED logic capture disables the skip: a scheduler event inside the
        // skipped window could toggle a watched pad, and the per-cycle poll
        // guarantee must hold even under this opt-in acceleration. Push-only
        // watch sets keep the skip — a pad write inside the window happens in
        // instrumented peripheral code, which pushes its own edge with the
        // post-skip tap clock (seeded below before the scheduler drain).
        if !self.config.idle_fast_forward_enabled || self.logic_capture.poll_active() {
            return 0;
        }
        // A cycle-accurate bus still skips a WFI wait while every resident is
        // idle (an idle DHT22 between reads). It does not coalesce timer polls:
        // that would leap a freehand `micros()` busy-loop, which is why such a
        // resident pins the bus in the first place.
        let cycle_accurate = self.bus.requires_cycle_accurate();
        if cycle_accurate
            && (self.bus.idle_skip_requires_cycle_accurate()
                || self.cpu.idle_fast_forward_budget(&self.bus).is_none())
        {
            return 0;
        }

        #[cfg(not(feature = "event-scheduler"))]
        {
            0
        }

        #[cfg(feature = "event-scheduler")]
        {
            if !self.bus.idle_fast_forward_legacy_safe() {
                return 0;
            }

            // Drain first: a due scheduler event may assert an interrupt, in
            // which case the CPU must resume normally instead of skipping.
            self.drain_scheduler_events();
            self.service_resident_edges_at_boundary();
            self.check_secondary_defer();
            let deferred_end = self.secondary_defer.map(|(_, end)| end);

            // Two coalesce sources under the same idle-FF flag:
            // 1) Architectural WFI / wait-for-interrupt (existing).
            // 2) Pure freerunning-timer poll batches (peripherals opt in via
            //    MmioAccessClass::FreerunningTimerPoll — e.g. ESP SYSTIMER
            //    snapshot regs for Arduino millis). Device time still advances
            //    to the next scheduled event; we only skip empty spin. Bus
            //    stays CPU-agnostic: chip register maps live in peripheral
            //    models, not SystemBus.
            // A dual-core machine is idle only when BOTH cores are. The skip
            // moves machine time without stepping either core, so a skip while
            // the secondary runs takes that time away from it: an ESP32 sketch
            // whose loopTask runs on the APP CPU while the PRO CPU sits in
            // WAITI got ~1 % of its cycles, and a CCOUNT-timed 20 ms servo
            // frame took ~2 s of machine time. A parked secondary keeps its
            // own wake deadline (CCOMPARE0) and its CCOUNT moves with the skip.
            //
            // A core busy in a proven cycle-counter poll loop (a CCOUNT
            // busy-wait, `Cpu::poll_loop_skip_budget`) counts as idle too: it
            // retires nothing but time until a known cycle, so skipping it is
            // the same as running it. The skip is then a multiple of its loop
            // period, and every core's timer edge clamps it.
            let remaining = _max_steps
                .map(|limit| limit.saturating_sub(_steps))
                .unwrap_or(1_000_000);
            if remaining == 0 {
                return 0;
            }
            let mut secondary_wake = None;
            let mut secondary_spin: Option<(u64, u64)> = None;
            if let Some(end) = deferred_end {
                // A deferred loop runs on through the skip; its deferral
                // already ends before its own timer edge.
                secondary_wake = Some(end - self.total_cycles);
            } else if let Some(sec) = self.cpu_secondary.as_mut() {
                match sec.secondary_execution_state() {
                    SecondaryExecutionState::Active => {
                        if !sec.poll_loop_anchor_ready() {
                            return 0;
                        }
                        match sec.poll_loop_skip_budget(&self.bus, remaining) {
                            Some(spin) => secondary_spin = Some(spin),
                            None => return 0,
                        }
                    }
                    SecondaryExecutionState::ParkedIdle => {
                        if sec.idle_fast_forward_budget(&self.bus).is_none() {
                            return 0;
                        }
                        secondary_wake = sec.parked_wake_deadline_cycles();
                    }
                    SecondaryExecutionState::ResetHeld => {}
                }
            }
            let wfi_budget = self.cpu.idle_fast_forward_budget(&self.bus);
            let timer_poll = !cycle_accurate && self.bus.take_timer_poll_coalesce_eligible();
            let primary_spin = match wfi_budget {
                Some(budget) if budget > 0 => None,
                _ if self.cpu.poll_loop_anchor_ready() => {
                    // The proof may only skip to the nearest deadline, so
                    // hand it that limit: a window too short for a whole
                    // iteration refuses before probing.
                    let mut limit = remaining;
                    let now = self.total_cycles;
                    for deadline in [
                        self.bus.next_cpu_interrupt_deadline_cycle(),
                        self.bus.next_motor_service_deadline_cycle(),
                        self.bus.next_resident_edge_deadline_cycle(),
                        self.sched.next_event_deadline(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        limit = limit.min(deadline.saturating_sub(now));
                    }
                    self.cpu.poll_loop_skip_budget(&self.bus, limit)
                }
                _ => None,
            };
            let spinning =
                primary_spin.is_some() || secondary_spin.is_some() || deferred_end.is_some();
            let mut budget = match (wfi_budget, primary_spin) {
                (Some(budget), _) if budget > 0 => budget,
                (_, Some((_, cycles))) => cycles,
                _ if timer_poll => u64::MAX,
                _ => return 0,
            };
            // The timer-poll arm below is only the one the budget came from.
            let timer_poll = timer_poll && wfi_budget.is_none() && primary_spin.is_none();
            budget = budget.min(remaining);
            if let Some(until) = secondary_wake {
                budget = budget.min(until);
            }
            if let Some((_, cycles)) = secondary_spin {
                budget = budget.min(cycles);
            }
            if spinning {
                // A spinning core's own CCOMPARE0 edge, and a WAITI primary's,
                // land on time: the window ends on the edge.
                if let Some(until) = self.cpu.parked_wake_deadline_cycles() {
                    budget = budget.min(until);
                }
                if let Some(until) = self
                    .cpu_secondary
                    .as_ref()
                    .and_then(|sec| sec.parked_wake_deadline_cycles())
                {
                    budget = budget.min(until);
                }
            }
            if let Some(deadline) = self.bus.next_cpu_interrupt_deadline_cycle() {
                budget = budget.min(deadline.saturating_sub(self.total_cycles));
            }
            if let Some(deadline) = self.bus.next_motor_service_deadline_cycle() {
                budget = budget.min(deadline.saturating_sub(self.total_cycles).max(1));
            }
            if let Some(deadline) = self.bus.next_resident_edge_deadline_cycle() {
                budget = budget.min(deadline.saturating_sub(self.total_cycles).max(1));
            }

            // A bus peripheral servicing an external medium (a medium-mode WiFi
            // MAC) must keep being pumped through the idle window or its inbound
            // frames are starved for the whole skip — breaking WiFi association
            // under fast-forward. Cap the skip to a small poll quantum and run
            // the bus-tick pump at the deadline (below, after the skip commits).
            // Gated on the tiny bus-tick set, so it is free on every other bus.
            let idle_poll = self.bus.idle_poll_bus_tick_active();
            if idle_poll {
                budget = budget.min(WIFI_MEDIUM_IDLE_POLL_QUANTUM);
            }

            if let Some(deadline_cycle) = self.sched.next_event_deadline() {
                if deadline_cycle <= self.total_cycles {
                    return 0;
                }
                budget = budget.min(deadline_cycle - self.total_cycles);
            } else if timer_poll && wfi_budget.is_none() {
                // No scheduled event and not WFI: cap pure millis spins so a
                // single batch cannot leap the whole step budget when the heap
                // is empty (still advances real device time, just bounded).
                budget = budget.min(1_000_000);
            }

            // Tiny windows are not worth the skip bookkeeping on the timer-poll
            // path (WFI may still skip short waits).
            if budget == 0 || (timer_poll && wfi_budget.is_none() && budget < 1024) {
                return 0;
            }
            budget = budget.min(u32::MAX as u64);
            if spinning {
                fn lcm_u64(a: u64, b: u64) -> u64 {
                    fn gcd(mut a: u64, mut b: u64) -> u64 {
                        while b != 0 {
                            (a, b) = (b, a % b);
                        }
                        a
                    }
                    if a == 0 || b == 0 {
                        return a.max(b);
                    }
                    (a / gcd(a, b)).saturating_mul(b)
                }
                // Whole loop iterations only, on every spinning core.
                let mut step = 1u64;
                for (period, _) in primary_spin.iter().chain(secondary_spin.iter()) {
                    step = lcm_u64(step, *period);
                }
                budget -= budget % step;
                if budget == 0 {
                    return 0;
                }
            }
            let skipped = budget as u32;
            if primary_spin.is_some() {
                let applied = self.cpu.fast_forward_poll_loop(skipped as u64);
                debug_assert!(applied, "a proven poll-loop skip must apply");
            } else {
                self.cpu.fast_forward_idle_cycles(skipped as u64);
            }
            if let Some(sec) = self
                .cpu_secondary
                .as_mut()
                .filter(|_| deferred_end.is_none())
            {
                if secondary_spin.is_some() {
                    let applied = sec.fast_forward_poll_loop(skipped as u64);
                    debug_assert!(applied, "a proven poll-loop skip must apply");
                } else if sec.secondary_execution_state() == SecondaryExecutionState::ParkedIdle {
                    sec.fast_forward_idle_cycles(skipped as u64);
                }
            }
            if spinning {
                self.poll_loop_cycles_skipped += skipped as u64;
                self.poll_loop_skips += 1;
            }
            self.total_cycles += skipped as u64;
            self.idle_fast_forward_cycles_skipped += skipped as u64;
            self.bus.set_current_cycle(self.total_cycles);
            // Motors are not scheduler heap entries. The budget above clamps
            // to their explicit service deadline, so service them at that
            // boundary before another idle skip can be planned.
            self.bus.service_motor_models();
            self.bus.bus_trace.set_cycle(self.total_cycles);
            // Push-mode logic capture: stamp any pad writes made by the
            // scheduler events due at the end of the skipped window with the
            // cycle we skipped to (the events' own deadline — the budget was
            // clamped to it above), keeping edges deterministic and correctly
            // placed inside what would otherwise be a silent window.
            if self.logic_capture.push_active() {
                self.bus.logic_tap.set_clock(self.total_cycles);
            }
            self.service_resident_edges_at_boundary();
            self.sched.advance_to(self.total_cycles);
            self.drain_scheduler_events();
            // Pump the external-medium bus-tick peripherals at the poll deadline
            // (current_cycle was published above), so a WiFi station pulls the
            // AP's queued auth/assoc/data frames and beacons on schedule even
            // though the CPU skipped the idle window. `idle_poll` bounded the
            // skip to the quantum, keeping this cadence fine-grained.
            if idle_poll {
                self.bus.run_idle_poll_bus_tick();
            }
            // Architectural idle still advances off-chip time. Without this
            // shared clock boundary a sleeping guest starves paced UART peers
            // and RX timeouts until unrelated CPU work resumes.
            self.advance_central_i2c_time();
            u64::from(skipped)
        }
    }

    /// Take a binary mid-flight runtime snapshot of the entire machine —
    /// CPU + every peripheral that overrides `runtime_snapshot()`. The
    /// returned blob feeds straight into [`Self::apply_runtime_snapshot`]
    /// on a freshly-constructed `Machine` with the same firmware loaded
    /// and the same bus topology.
    ///
    /// `None` when this machine's CPU models no runtime snapshot
    /// ([`Cpu::runtime_snapshot`]). A machine-level snapshot whose CPU half
    /// is missing is not a cheaper snapshot, it is a resume that silently
    /// starts from a cold core with warm RAM — so there is no such value to
    /// return. Callers that used to gate on a separate capability query now
    /// simply handle the `None`, which cannot disagree with what a capture
    /// would actually contain.
    ///
    /// Distinct from [`Self::snapshot`] (which produces a JSON value for
    /// the determinism gates) — this one is binary, captures everything
    /// needed for resume (full SR file, shadow stacks, RAM regions, etc.)
    /// and is what the playground uses to ship pre-warmed boot snapshots
    /// alongside firmware ELFs.
    pub fn take_runtime_snapshot(&self) -> Option<runtime_snapshot::MachineRuntimeSnapshot> {
        let (cpu_kind, cpu_data) = self.cpu.runtime_snapshot()?;
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
        let mut snap =
            runtime_snapshot::MachineRuntimeSnapshot::new(cpu_kind, cpu_data, peripherals);
        // RISC-V (ESP32-C3 rom-boot) keeps its live program state — `.data`,
        // `.bss`, stacks — in the flat SRAM/IRAM linear windows (`bus.ram` +
        // `bus.extra_mem`), NOT in RamPeripherals like the Xtensa configs. So a
        // faithful resume must carry those windows. Xtensa/Arm snapshots leave
        // `memories` empty: their RAM is peripheral-backed (already captured
        // above) and their linear windows are firmware mirrors re-derived on
        // load. Untouched (all-zero) extra_mem windows — e.g. the flash-mapped
        // DROM mirror — are skipped so the blob stays compact.
        if cpu_kind == runtime_snapshot::CpuKind::RiscV {
            let mut memories = vec![(self.bus.ram.base_addr, self.bus.ram.data.clone())];
            for mem in &self.bus.extra_mem {
                if mem.data.iter().any(|&b| b != 0) {
                    memories.push((mem.base_addr, mem.data.clone()));
                }
            }
            snap.memories = memories;
        }
        Some(snap)
    }

    /// Restore from a previously-taken runtime snapshot. Bus topology
    /// must match (same peripherals registered under the same names) —
    /// peripherals not present in the snapshot keep their current state,
    /// peripherals named in the snapshot but missing on the bus return
    /// `MissingPeripheral` so the caller can fail loudly instead of
    /// silently dropping state.
    ///
    /// The CPU half is restored FIRST and its error is propagated, so a core
    /// that models no restore ([`Cpu::apply_runtime_snapshot`]'s default)
    /// fails the whole call before a single peripheral is touched. That
    /// ordering is deliberate: a partial restore that reported `Ok(())` gave
    /// back a machine with warm peripherals and a cold CPU.
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
            // Older ROM-boot captures could contain an empty declarative
            // placeholder followed by the behavioral device under the same
            // name. After that bus entry is replaced in place, only the real
            // state belongs to it. Keep singleton-empty and malformed state
            // validation, and still require even a placeholder's device above.
            if blob.is_empty()
                && snap
                    .peripherals
                    .iter()
                    .any(|(other_name, other_blob)| other_name == name && !other_blob.is_empty())
            {
                continue;
            }
            entry.dev.restore_runtime_snapshot(blob)?;
        }
        // Restore flat linear windows (RISC-V SRAM/IRAM; empty for other
        // arches). Match each captured window to its live backing by base
        // address and fail loudly on a topology/size mismatch — a resume must
        // be applied on the same machine layout it was captured on.
        for (base, bytes) in &snap.memories {
            let target = if *base == self.bus.ram.base_addr {
                Some(&mut self.bus.ram)
            } else {
                self.bus.extra_mem.iter_mut().find(|m| m.base_addr == *base)
            };
            let mem = target.ok_or_else(|| {
                SimulationError::NotImplemented(format!(
                    "apply_runtime_snapshot: no memory window @ {base:#010x} on bus"
                ))
            })?;
            if mem.data.len() != bytes.len() {
                return Err(SimulationError::NotImplemented(format!(
                    "apply_runtime_snapshot: memory @ {base:#010x} size {} != snapshot {}",
                    mem.data.len(),
                    bytes.len()
                )));
            }
            mem.data.copy_from_slice(bytes);
        }
        self.bus.refresh_peripheral_index();
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
            //
            //    This is a host/loader write, not a CPU store: the same
            //    contract the direct flash/ram paths already satisfy. A CPU
            //    store policy (`cpu_writes: fault`) or an armed debug watch
            //    must not reject or record it — a boot-alias image linked at
            //    0 on a chip whose flash lives at `0x0800_0000` lands here.
            let all_written = self.bus.with_debug_watch_bypass(|bus| {
                bus.with_memory_policy_bypass(|bus| {
                    let mut all_written = true;
                    for (i, byte) in segment.data.iter().enumerate() {
                        let addr = segment.start_addr + i as u64;
                        if bus.write_u8(addr, *byte).is_err() {
                            all_written = false;
                            break;
                        }
                    }
                    all_written
                })
            });
            if !all_written {
                if self.cpu.uses_elf_entry_point() {
                    return Err(SimulationError::MemoryViolation(segment.start_addr));
                }
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

        if self.cpu.uses_elf_entry_point() {
            let entry = u32::try_from(image.entry_point)
                .map_err(|_| SimulationError::MemoryViolation(image.entry_point))?;
            self.cpu.set_pc(entry);
            return Ok(());
        }

        // Resolve the reset vector.
        //
        // `cpu.reset` reads (SP, PC) from VTOR (default 0). On STM32/nRF that
        // aliases the flash image, so reset is already correct. On RP2040 the
        // in-tree mask ROM is mapped at 0 (`LABWIRED_RP2040_BOOTROM` default),
        // so reset installs *bootrom* vectors and would park the core in the
        // ROM WFI loop forever for bare-metal images that put a valid vector
        // table at `flash_base` (no stage-2 boot2). Prefer flash-base vectors
        // when they are valid; only then fall back to post-boot2 relocation.
        let flash_base = self.bus.flash.base_addr;
        if self.bus.vector_pair_valid(flash_base) {
            let sp = self.bus.read_u32(flash_base)?;
            let pc = self.bus.read_u32(flash_base + 4)? & !1;
            // If address 0 does not already present the same SP (bootrom or
            // empty), retarget VTOR so exceptions hit the app table too.
            let sp0 = self.bus.read_u32(0).unwrap_or(0);
            if flash_base != 0 && sp0 != sp {
                let _ = self.bus.write_u32(0xE000_ED08, flash_base as u32);
            }
            self.cpu.set_sp(sp);
            self.cpu.set_pc(pc);
        } else {
            // Stage-2 present (e.g. RP2040 boot2 at flash[0..0x100]): vectors
            // at flash_base are garbage. Relocate to `reset_vector_offset`.
            let offset = self.bus.reset_vector_offset;
            let relocated = if offset != 0 {
                let table = flash_base + offset;
                if self.bus.vector_pair_valid(table) {
                    let sp = self.bus.read_u32(table)?;
                    let pc = self.bus.read_u32(table + 4)? & !1;
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

        // The VTOR a system reset returns to (see `boot_vtor`).
        if self.scb_index.is_some() {
            self.boot_vtor = self.bus.read_u32(0xE000_ED08).ok();
        }

        Ok(())
    }

    /// The core-local half of a Cortex-M system reset (AIRCR.SYSRESETREQ),
    /// applied before the CPU reloads MSP/PC: VTOR back to its boot value, the
    /// NVIC's enable/pending/active/priority state cleared, SysTick stopped,
    /// and the SCB's SCR/SHPR1-3 zeroed — all of which the architecture
    /// resets. Peripheral models are NOT reset (there is no per-peripheral
    /// reset hook yet); with every NVIC enable cleared their stale events
    /// cannot interrupt the new boot until firmware re-enables the line.
    ///
    /// Without this, a CODAL micro:bit V2 image — which reboots itself twice
    /// on first boot after programming UICR (NFCPINS, REGOUT0) — came back
    /// with VTOR still pointing at the SRAM vector table the previous boot had
    /// installed. Startup zeroes .bss (that table included), the still-enabled
    /// TIMER IRQs fired, and the core vectored to 0x00000000.
    fn reset_core_system_state(&mut self) {
        if self.scb_index.is_none() {
            return;
        }
        if let Some(vtor) = self.boot_vtor {
            let _ = self.bus.write_u32(0xE000_ED08, vtor);
        }
        if let Some(nvic) = &self.bus.nvic {
            use std::sync::atomic::Ordering;
            for word in nvic
                .iser
                .iter()
                .chain(nvic.ispr.iter())
                .chain(nvic.iabr.iter())
                .chain(nvic.level_pended.iter())
                .chain(nvic.ipr.iter())
            {
                word.store(0, Ordering::SeqCst);
            }
            nvic.sev_on_pend.store(false, Ordering::SeqCst);
            nvic.event_register.store(false, Ordering::SeqCst);
        }
        // SysTick CSR (ENABLE/TICKINT/CLKSOURCE) and the SCB registers the
        // architecture resets to zero: SCR, SHPR1, SHPR2, SHPR3.
        for addr in [
            0xE000_E010u64,
            0xE000_ED10,
            0xE000_ED18,
            0xE000_ED1C,
            0xE000_ED20,
        ] {
            let _ = self.bus.write_u32(addr, 0);
        }
    }

    pub fn reset(&mut self) -> SimResult<()> {
        self.cpu.reset(&mut self.bus)?;
        if let Some(cpu1) = self.cpu_secondary.as_mut() {
            cpu1.reset(&mut self.bus)?;
        }
        Ok(())
    }

    /// The supply supervisor's view of this machine: whether a circuit drives
    /// VDD, its last value, whether the core is held in reset, and how many
    /// power-on / brown-out resets it has come out of. See [`crate::power`].
    pub fn supply_status(&self) -> crate::power::SupplyStatus {
        self.bus.supply.status()
    }

    /// A circuit drives this machine's VDD from now on: hold the core in
    /// reset until the supply delivers. [`crate::cosim::CosimSession`] calls
    /// it for a manifest that routes `board.power.vdd_volts`; a caller that
    /// drives VDD itself (a test) calls it and then [`Self::set_supply_volts`].
    pub fn attach_supply(&mut self) {
        self.bus.supply.attach();
    }

    /// Feed VDD by hand, volts. A no-op until [`Self::attach_supply`].
    pub fn set_supply_volts(&mut self, volts: f64) {
        self.bus.supply.set_vdd(volts);
    }

    /// Act on a release the supply supervisor has decided: restart the core
    /// through its reset vector (what `SYSRESETREQ` does) and let each
    /// peripheral record the reset cause.
    fn apply_supply_release(&mut self) -> SimResult<()> {
        let Some(cause) = self.bus.supply.take_release() else {
            return Ok(());
        };
        self.reset()?;
        for peripheral in &mut self.bus.peripherals {
            peripheral.dev.on_supply_reset(cause);
        }
        tracing::debug!("supply supervisor released the core: {}", cause.as_str());
        Ok(())
    }

    /// Advances one primary-CPU boundary through the authoritative lifecycle.
    ///
    /// This compatibility adapter delegates to [`Machine::advance`]. Frontends
    /// that need bounded runs or stop reports should call `advance` directly;
    /// they must not reproduce the lifecycle with direct [`Cpu::step`] calls.
    pub fn step(&mut self) -> SimResult<()> {
        self.advance(AdvanceRequest::single()).map(|_| ())
    }

    /// Drain and apply the single pending H5 FLASH hardware operation, if any.
    ///
    /// The FLASH peripheral records at most one op per instruction (in a `Cell`);
    /// this helper must therefore run once per instruction so no op is lost. It
    /// is called from the authoritative `Machine::advance` batch loop. The
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
                    let (mut bank_size, sector_size) = self
                        .flash_peripheral()
                        .map(|f| f.flash_geometry())
                        .unwrap_or((h5::BANK_SIZE, h5::SECTOR_SIZE));
                    // STM32U5 flash is always two banks (RM0456 §7;
                    // `FLASH_BANK_SIZE = FLASH_SIZE >> 1` in stm32u545xx.h): a
                    // 2 MiB U575 has 1 MiB banks (the constant above), a 512 KiB
                    // U545 has 256 KiB banks. Derive it from the mapped flash so a
                    // BKER page erase lands in the right half. Gated on the U5
                    // layout, so no other family's geometry changes.
                    if self
                        .flash_peripheral()
                        .is_some_and(|f| f.u5_error_flags_enabled())
                    {
                        bank_size = self.bus.flash.data.len() as u64 / 2;
                    }
                    let in_flash = (h5::FLASH_BASE..h5::FLASH_BASE + 2 * bank_size).contains(&pc);
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
                    let offset = (bank as u64) * bank_size + (sector as u64) * sector_size;
                    self.bus.flash.fill(offset, sector_size, 0xFF);
                    tracing::debug!(
                        "FLASH EraseSector bank={bank} sector={sector} offset={offset:#010x}"
                    );
                }
                FlashOp::SwapAndReset => {
                    // Swap the two architectural 1 MiB (0x100000) banks. The
                    // H563 flash buffer is sized to exactly 2 * BANK_SIZE by the
                    // chip yaml (`size: 2MiB`), so the same BANK_SIZE used by
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
    /// peripheral event. The authoritative [`Machine::advance`] lifecycle
    /// calls this after each committed CPU batch so no scheduler-driven
    /// peripheral is silently stranded.
    ///
    /// The scheduler runs in absolute CPU cycles (`total_cycles`) — the same
    /// quantum the legacy walk advances by (`tick_elapsed(interval)`) — so
    /// cycle-denominated peripheral delays (an SPI half-period, a systimer
    /// alarm) keep their exact meaning at any `peripheral_tick_interval`. An
    /// event lands at the first drain at or after its exact cycle; drains run
    /// at least once per CPU batch, so the observation error is bounded by one
    /// tick interval (and is zero at interval 1, where drains run per cycle).
    /// Write-context schedule requests the bus buffered during the committed
    /// batch's MMIO writes (`pending_schedule`) are enqueued first: a
    /// peripheral can't reach the scheduler from `write`, so the bus buffers
    /// an absolute cycle deadline (see `collect_scheduled_events`) which is
    /// clamped to `now` here — a deadline that expired mid-batch fires on this
    /// drain.
    #[cfg(feature = "event-scheduler")]
    fn drain_scheduler_events(&mut self) {
        // Measured INCLUSIVE of the peripheral handlers dispatched below;
        // `profile::Snapshot::report` subtracts them so nothing is charged
        // twice.
        let span = crate::profile::span();
        if span.is_some() {
            // Stable and unique among live machines for as long as this one
            // exists, which is all the report needs to tell "one chip" from
            // "several merged".
            crate::profile::set_machine(self as *const Self as u64);
        }
        self.drain_scheduler_events_inner();
        crate::profile::record_sched(span);
    }

    #[cfg(feature = "event-scheduler")]
    fn drain_scheduler_events_inner(&mut self) {
        // One-time bootstrap: give every scheduler-driven peripheral a chance
        // to schedule events that arise from *setup* rather than an MMIO write
        // (e.g. a UART with an RX stream attached before firmware advances, or
        // a SYSTIMER whose alarm was configured before `Machine::advance`). The
        // absolute deadline is `total_cycles + delay`, which is only exact if
        // the returned delay is measured from `total_cycles` — but a peripheral
        // is anchored at ATTACH (cycle 0) and the first drain can run after
        // `Machine::advance` has committed its first batch and advanced
        // `total_cycles`. Sync each
        // peripheral up to `total_cycles` FIRST so its delay is genuinely
        // relative to now; without this the first scheduled event lands one (or
        // up to one tick-interval) cycle late versus the legacy per-cycle walk —
        // the exact off-by-one the ESP32-S3 `intmatrix_alarm`/walk-differential
        // gate caught on a pre-advance alarm config. `sync_to` is idempotent at
        // cycle 0 (no-op) and the same call the write path already makes, so
        // steady-state behaviour is unchanged.
        if !self.scheduler_bootstrapped {
            self.scheduler_bootstrapped = true;
            let now = self.total_cycles;
            for idx in 0..self.bus.peripherals.len() {
                if self.bus.peripherals[idx].dev.uses_scheduler() {
                    self.bus.peripherals[idx].dev.sync_to(now);
                    for (delay, token) in self.bus.peripherals[idx].dev.take_scheduled_events() {
                        self.bus.pending_schedule.push((idx, now + delay, token));
                    }
                }
            }
        }
        self.sched.advance_to(self.total_cycles);
        let now = self.sched.now();
        // Swap the buffered schedule out into retained scratch (instead of
        // `mem::take`, which frees the buffer's capacity each drain) and drain
        // it in place — same entries, same order, but the capacity is reused.
        std::mem::swap(
            &mut self.bus.pending_schedule,
            &mut self.pending_schedule_scratch,
        );
        for (idx, deadline, token) in self.pending_schedule_scratch.drain(..) {
            self.sched.schedule(deadline.max(now), idx as u32, token);
        }
        // Nothing queued (steady state between peripheral events):
        // skip the heap drain entirely — no allocation.
        if self.sched.is_empty() {
            return;
        }
        // Fill the retained scratch (taken out so `on_event` below can borrow
        // `&mut self.sched` / `&mut self.bus`) instead of allocating a fresh
        // `Vec` per drain; restored at the end with its capacity intact.
        let mut due = std::mem::take(&mut self.due_events_scratch);
        self.sched.drain_due_into(&mut due);
        for ev in due.drain(..) {
            let idx = ev.peripheral_idx as usize;
            if idx >= self.bus.peripherals.len() {
                continue;
            }
            if self.bus.peripherals[idx].dev.requires_clocked_tick()
                && !self.bus.is_peripheral_clocked(idx)
            {
                self.sched
                    .schedule(now + 1, ev.peripheral_idx, ev.event_token);
                continue;
            }
            // Swap the peripheral out so we can pass `&mut self.bus` into
            // `on_event` without holding two simultaneous mutable borrows.
            // Same dance the bus uses for `tick_with_bus`, but the stub stand-in
            // is reused from `event_placeholder` instead of allocated per event.
            let stub = self
                .event_placeholder
                .take()
                .expect("event_placeholder present between events");
            let mut dev = std::mem::replace(&mut self.bus.peripherals[idx].dev, stub);
            // Attribute the handler to the peripheral that owns it. This is the
            // measurement that told us `i2c0` is 18% of the BLE Pong lab while
            // the `bt` model — the subsystem the slowdown was blamed on — is
            // 1.6%. Event COUNTS say the opposite; see `profile`'s header.
            let handler_span = crate::profile::span();
            let result = dev.on_event(ev.event_token, &mut self.sched, &mut self.bus);
            crate::profile::record_peripheral(handler_span, idx);
            // Put the real peripheral back and reclaim the stub for reuse.
            let stub_back = std::mem::replace(&mut self.bus.peripherals[idx].dev, dev);
            self.event_placeholder = Some(stub_back);
            // Phase 2B.3b: a level-triggered peripheral re-arms its own event
            // (same token) while it has active work. We own the idx the
            // scheduler needs, so we do it here.
            if let Some(delay) = result.reschedule_delay {
                let deadline = self.sched.now() + delay;
                self.sched.schedule(deadline, idx as u32, ev.event_token);
            }
            self.apply_event_result(idx, result);
        }
        // `drain(..)` above emptied it but kept its capacity; restore for reuse.
        self.due_events_scratch = due;
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
                // A LEVEL source's event-path pend is marked, so the MMIO
                // write choke can drop it again when the handler clears the
                // status flag — scheduler-driven timers never pass through
                // the walk's reconcile.
                if self.bus.peripherals[peripheral_idx]
                    .dev
                    .irq_line_level()
                    .is_some()
                {
                    crate::bus::reconcile_nvic_level(&self.bus.nvic, irq, true);
                }
            }
        }
        // Scheduler-driven interrupt delivery — ONE per-fabric choke. A
        // scheduler-driven peripheral (e.g. the SYSTIMER alarm) is skipped by
        // the per-cycle walk, so the event path owns delivery of its
        // LEVEL-sensitive source at the exact firing cycle. Every MCU family
        // follows the same shape behind `deliver_scheduled_irq_levels`:
        //   * ESP32-C3 (RISC-V matrix)  → re-derive `matrix_irq_sources` into
        //     `irq_fabric.esp32c3.irq_lines`;
        //   * ESP32-S3 (Xtensa intmatrix) → re-derive into `pending_cpu_irqs` +
        //     the intmatrix INTR_STATUS mirror.
        // A matrix source ID must NEVER be pended as a Cortex-M NVIC exception
        // (`pend_irq_for_event` would mis-route it), so the NVIC fallthrough is
        // taken only when no matrix fabric claimed delivery (the Cortex-M /
        // nRF SysTick + own-line path).
        if !self.bus.deliver_scheduled_irq_levels() {
            for irq in &result.explicit_irqs {
                self.bus.pend_irq_for_event(*irq, &mut fallthrough);
            }
            // A multi-line LEVEL source (STM32 EXTI) marks the pends its event
            // chain raises — scheduler-driven models never pass through the
            // walk's reconcile — so the MMIO write choke can drop them when
            // the handler clears the pending latch.
            if !result.explicit_irqs.is_empty() {
                self.bus.mark_multi_irq_levels(peripheral_idx);
            }
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
        // Each item runs on behalf of its issuer, which hears about a FIFO
        // load's data and about a bus error (`Peripheral::dma_load_data` /
        // `dma_transfer_error`).
        for req in &result.dma_requests {
            self.bus.execute_dma_for(peripheral_idx, req);
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

    /// True while RTC_CNTL has a latched SW_SYS_RST (not yet drained).
    fn rtc_cntl_reset_pending(&self) -> bool {
        let Some(idx) = self.rtc_cntl_index else {
            return false;
        };
        let Some(p) = self.bus.peripherals.get(idx) else {
            return false;
        };
        p.dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::rtc_cntl::RtcCntl>())
            .map(|rtc| rtc.reset_request_pending())
            .unwrap_or(false)
    }

    /// Drain every reset-capable peripheral's latch. Every capable peripheral
    /// is polled (no short-circuit) so each latch clears even when another
    /// already fired. The returned causes are what was latched before the
    /// take; an empty vec means nobody requested a reset.
    pub(crate) fn drain_peripheral_reset_requests(&mut self) -> Vec<Option<PeripheralResetCause>> {
        let mut requests = Vec::new();
        for i in 0..self.reset_request_indices.len() {
            let idx = self.reset_request_indices[i];
            let cause = self.bus.peripherals[idx].dev.latched_reset_cause();
            if self.bus.peripherals[idx].dev.take_reset_request() {
                requests.push(cause);
            }
        }
        requests
    }

    /// Tell every peripheral that a CPU reboot from `cause` is being applied.
    fn record_peripheral_reset_cause(&mut self, cause: PeripheralResetCause) {
        for peripheral in &mut self.bus.peripherals {
            peripheral.dev.on_peripheral_reset(cause);
        }
    }

    /// Returns true (and clears the latch) if the registered SCB peripheral
    /// has a pending SYSRESETREQ — an AIRCR write with the correct VECTKEY and
    /// the SYSRESETREQ bit set (`Scb::write_reg`, offset 0x0C). The
    /// authoritative [`Machine::advance`] lifecycle drains this latch at a
    /// clean committed instruction boundary. Uses the cached `scb_index`
    /// resolved at construction — non-Cortex-M configs (no SCB on the bus)
    /// short-circuit to `false` without touching the peripheral vector at all.
    pub(crate) fn drain_scb_reset_request(&self) -> bool {
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

    /// Take the exit code firmware wrote to the `simctl` device, if any.
    ///
    /// Returns `None` immediately on a bus with no `simctl` — the cached index
    /// makes this a single `Option` test rather than a peripheral walk, so the
    /// advance loop pays effectively nothing for a feature it is not using.
    pub(crate) fn drain_simctl_exit_code(&self) -> Option<u32> {
        let idx = self.simctl_index?;
        self.bus
            .peripherals
            .get(idx)?
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::simctl::SimCtl>())
            .and_then(|d| d.drain_exit_code())
    }

    /// Borrow the `simctl` device, if the bus declares one. Lets a harness read
    /// the `SOUT`/`SERR` streams after a run ends.
    pub fn simctl(&self) -> Option<&crate::peripherals::simctl::SimCtl> {
        let idx = self.simctl_index?;
        self.bus
            .peripherals
            .get(idx)?
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<crate::peripherals::simctl::SimCtl>())
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
        self.cpu.try_apply_snapshot(&snapshot.cpu)?;
        for p in &mut self.bus.peripherals {
            if let Some(state) = snapshot.peripherals.get(&p.name) {
                p.dev.restore(state.clone())?;
            }
        }
        self.bus.refresh_peripheral_index();
        Ok(())
    }

    pub fn peek_peripheral(&self, name: &str) -> Option<serde_json::Value> {
        self.bus
            .peripherals
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.dev.snapshot())
    }

    /// Universal inspect: enumerate + decode peripheral state (snapshot
    /// semantics — reads the current paused/post-run machine). `name = Some`
    /// (or `opts.peripheral`) restricts the walk to one peripheral; `None`
    /// inspects all. Decode is side-effect-free (uses `peek`, never `read`).
    /// See [`crate::inspect`].
    pub fn inspect(
        &self,
        name: Option<&str>,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::MachineInspect {
        let filter = name.or(opts.peripheral.as_deref());
        let peripherals = self
            .bus
            .peripherals
            .iter()
            .filter(|entry| filter.is_none_or(|f| entry.name == f))
            .map(|entry| {
                let inspected = entry.dev.inspect(entry.base, &entry.name, opts);
                // A peripheral that describes itself always wins — the schema is
                // a fallback for NATIVE peripherals, which have none, and must
                // never override a model's own account of its registers.
                if !inspected.registers.is_empty() {
                    return inspected;
                }
                match self.bus.debug_schemas.get(&entry.name) {
                    Some(schema) => {
                        let mut named = crate::inspect::inspect_with_schema(
                            entry.dev.as_ref(),
                            entry.base,
                            &entry.name,
                            schema,
                        );
                        // Preserve any artifacts (framebuffers, uart rings, pin
                        // state) the peripheral produced; only its registers
                        // were missing.
                        named.artifacts = inspected.artifacts;
                        named
                    }
                    None => inspected,
                }
            })
            .collect();
        // Device binding lives on the bus, which is the one thing that holds
        // BOTH the live models (inside controllers and directly on itself) and
        // the manifest declarations they are named from. See
        // `crate::bus::attached_devices`.
        let devices = self.bus.inspect_devices(filter, opts);
        crate::inspect::MachineInspect {
            peripherals,
            devices,
        }
    }

    /// Raw escape hatch: read `len` bytes at absolute `addr`, side-effect-free.
    /// Clamps to mapped regions — bytes outside any memory region or peripheral
    /// window come back as [`crate::inspect::PeekByte::Unmapped`] rather than a
    /// silent zero, so unmodeled space is never mistaken for real data.
    pub fn peek(&self, addr: u64, len: usize) -> crate::inspect::PeekResult {
        let mut bytes = Vec::with_capacity(len);
        for i in 0..len as u64 {
            bytes.push(match self.bus.peek_byte(addr + i) {
                Some(v) => crate::inspect::PeekByte::Mapped(v),
                None => crate::inspect::PeekByte::Unmapped,
            });
        }
        crate::inspect::PeekResult { addr, bytes }
    }
}

impl<C: Cpu> DebugControl for Machine<C> {
    fn fault_capture(&self) -> Option<fault_verdict::FaultCapture> {
        self.cpu.fault_capture()
    }

    fn add_breakpoint(&mut self, addr: u32) {
        self.breakpoints.insert(addr);
    }

    fn remove_breakpoint(&mut self, addr: u32) {
        self.breakpoints.remove(&addr);
    }

    fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    fn add_watchpoint(&mut self, base: u64, len: u64, on_read: bool, on_store: bool) -> bool {
        self.bus.add_debug_watch(base, len, on_read, on_store)
    }

    fn remove_watchpoint(&mut self, base: u64, len: u64, on_read: bool, on_store: bool) -> bool {
        self.bus.remove_debug_watch(base, len, on_read, on_store)
    }

    fn clear_watchpoints(&mut self) {
        self.bus.clear_debug_watches();
    }

    fn run(&mut self, max_steps: Option<u32>) -> SimResult<StopReason> {
        // Defensive: a watch the host removed can leave its last latch behind
        // (only `clear_watchpoints` drops a pending hit); with no watch armed
        // it must not stop this run.
        if self.bus.debug_watches.is_empty() {
            let _ = self.bus.take_debug_watch_hit();
        }
        let report = self.advance(AdvanceRequest::run(max_steps.map(u64::from)))?;
        // `advance_inner` consumes the latch into `AdvanceStop::Watchpoint` at
        // the window boundary. This catches a hit latched by a window that
        // ended the run for another reason (firmware exit, net drive change)
        // before the next boundary could see it — the access happened, so the
        // watch is the stop.
        if let Some(addr) = self.bus.take_debug_watch_hit() {
            return Ok(StopReason::Watchpoint(addr));
        }
        Ok(match report.stop {
            AdvanceStop::Watchpoint(addr) => StopReason::Watchpoint(addr),
            AdvanceStop::Breakpoint(pc) => StopReason::Breakpoint(pc),
            AdvanceStop::FuelLimit => StopReason::MaxStepsReached,
            AdvanceStop::CycleLimit | AdvanceStop::NoProgress | AdvanceStop::NetDriveChange => {
                StopReason::StepDone
            }
            AdvanceStop::FirmwareExit { code } => StopReason::FirmwareExit(code),
        })
    }
    fn step_single(&mut self) -> SimResult<StopReason> {
        if self.bus.debug_watches.is_empty() {
            let _ = self.bus.take_debug_watch_hit();
        }
        let report = self.advance(AdvanceRequest::single())?;
        if let Some(addr) = self.bus.take_debug_watch_hit() {
            return Ok(StopReason::Watchpoint(addr));
        }
        Ok(match report.stop {
            AdvanceStop::Watchpoint(addr) => StopReason::Watchpoint(addr),
            _ => StopReason::StepDone,
        })
    }

    fn read_core_reg(&self, id: u8) -> u32 {
        self.cpu.get_register(id)
    }

    fn write_core_reg(&mut self, id: u8, val: u32) {
        self.cpu.set_register(id, val);
    }

    fn read_memory(&self, addr: u32, len: usize) -> SimResult<Vec<u8>> {
        // Debugger-context read: it must not latch a read watch — a GDB `x`
        // must not fire a watchpoint. The flag is restored on every path
        // (the `try_for_each` collects the first error and the restore runs
        // after it), and the `Cell` is what makes this possible on `&self`.
        // The loader reads flash directly, never through this function.
        let prev = self.bus.debug_watch_host_bypass.replace(true);
        let mut data = Vec::with_capacity(len);
        let result = (0..len).try_for_each(|i| {
            let byte = self.bus.read_u8((addr as u64) + (i as u64))?;
            data.push(byte);
            Ok(())
        });
        self.bus.debug_watch_host_bypass.set(prev);
        result.map(|()| data)
    }

    fn write_memory(&mut self, addr: u32, data: &[u8]) -> SimResult<()> {
        // Debugger-context write: it must not latch a store watch. The flag
        // is restored on every path (the `try_for_each` collects the first
        // error and the restore runs after it). The loader (`load_firmware`)
        // does not go through this function: its direct paths write
        // LinearMemory and its peripheral-backed fallback walks the bus under
        // its own bypass flags.
        let prev = self.bus.debug_watch_host_bypass.replace(true);
        let result = data
            .iter()
            .enumerate()
            .try_for_each(|(i, byte)| self.bus.write_u8((addr as u64) + (i as u64), *byte));
        self.bus.debug_watch_host_bypass.set(prev);
        result
    }

    // Both forward to the inherent methods of the same name. Exposing them on
    // the trait is what lets a `Box<dyn DebugControl>` holder (the DAP server)
    // reach the side-effect-free surface that MCP and the playground already use.
    fn inspect(
        &self,
        name: Option<&str>,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::MachineInspect {
        Machine::inspect(self, name, opts)
    }

    fn peek(&self, addr: u64, len: usize) -> crate::inspect::PeekResult {
        Machine::peek(self, addr, len)
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
        let entry = self.bus.peripherals.iter().find(|p| p.name == name)?;
        entry.dev.peripheral_descriptor()
    }

    fn reset(&mut self) -> SimResult<()> {
        Machine::reset(self)
    }

    fn snapshot(&self) -> snapshot::MachineSnapshot {
        self.snapshot()
    }

    fn restore(&mut self, snapshot: &snapshot::MachineSnapshot) -> SimResult<()> {
        self.apply_snapshot(snapshot.clone())
    }
}

/// A fast, dependency-free hasher for small integer keys (register offsets).
///
/// Peripheral register banks are `HashMap<u64, u32>` keyed by offset and are
/// read on nearly every tick — `UartCore::int_raw` alone walks several. With
/// std's default SipHash, `sip::Hasher::write` profiled as the SECOND heaviest
/// frame in a whole classic-ESP32 run, behind only the peripheral-walk loop.
/// SipHash is a DoS-resistant hash for adversarial keys; a register offset the
/// firmware chose is not that.
///
/// This is the same finding, and the same fix, that took the ESP32-C3 labs
/// 2.343s -> 1.955s (see the scheduler-hash work). Multiply-xor, i.e. the FxHash
/// construction, without taking a new dependency.
#[derive(Default, Clone, Copy)]
pub struct FastHasher(u64);

impl std::hash::Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u8(b);
        }
    }
    #[inline]
    fn write_u8(&mut self, n: u8) {
        self.write_u64(u64::from(n));
    }
    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.write_u64(u64::from(n));
    }
    #[inline]
    fn write_u64(&mut self, n: u64) {
        // FxHash: rotate, xor, multiply by a large odd constant.
        const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(SEED);
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

/// Drop-in replacement for `HashMap` on integer keys. Same semantics, same
/// iteration-order guarantees (i.e. none), just a cheaper hash.
pub type FastMap<K, V> = std::collections::HashMap<K, V, std::hash::BuildHasherDefault<FastHasher>>;
