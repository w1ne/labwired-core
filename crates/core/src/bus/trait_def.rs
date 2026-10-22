// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The bus contract seen by CPU cores and peripherals.
//!
//! Kept out of the crate root so the entire MMIO/DMA/interrupt/scheduler
//! surface has one home. The crate root and `bus` re-export both public items,
//! so existing `labwired_core::Bus` and `crate::FetchPermission` paths continue
//! to resolve unchanged.

use std::any::Any;

use crate::{logic_capture, DmaRequest, Peripheral, SimResult, SimulationConfig, SimulationError};

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
    /// Default: no bypass. See [`crate::with_device_write_bypass`].
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
