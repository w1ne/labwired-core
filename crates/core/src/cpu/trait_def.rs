// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The `Cpu` trait and the types its implementors share.
//!
//! Kept out of the crate root so the execution contract has one home. The
//! crate re-exports every public item below, so `labwired_core::Cpu` and the
//! documented `crate::` paths continue to resolve unchanged.

use std::any::Any;
use std::sync::Arc;

use crate::snapshot;
use crate::{
    fault_verdict, runtime_snapshot, Bus, SimResult, SimulationConfig, SimulationError,
    SimulationObserver,
};

/// Feature-agnostic JIT engine run counters, mirrored from a concrete CPU's
/// engine so generic (`C: Cpu`) callers can observe non-vacuity without pulling
/// in the JIT-only `EngineStats` type. All-zero / absent for interpreter-only
/// CPUs and for runs where the JIT engine was never created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuJitStats {
    /// Blocks that crossed the hot threshold and were compiled + installed.
    pub compiled: u64,
    /// Compiled-block invocations.
    pub block_runs: u64,
    /// Guest instructions retired inside compiled blocks.
    pub block_instrs: u64,
    /// Guest instructions retired on the interpreter fallback path.
    pub interpreted: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecondaryExecutionState {
    Active,
    ParkedIdle,
    ResetHeld,
}

/// The body of [`Cpu::step_batch`]'s default, as a free function so a core that
/// needs to bracket the batch (set up state that is only valid while nothing
/// but its own `step` runs) can wrap it instead of copying it. Copying would
/// fork the loop and add `event-scheduler` cfg sites; see `XtensaLx7::step_batch`.
// `inline(always)` is load-bearing, and measured: without it, moving this body
// out of the trait default shifted codegen in unrelated bus-tick code, and
// Core Perf 36067877866 put +0.4..+1.0% on every nRF/EFR32 step-mode board.
// With it (36072477356) every board is back within +/-0.06%.
#[inline(always)]
pub(crate) fn default_step_batch<C: Cpu + ?Sized>(
    cpu: &mut C,
    bus: &mut dyn Bus,
    observers: &[Arc<dyn SimulationObserver>],
    config: &SimulationConfig,
    max_count: u32,
) -> SimResult<u32> {
    default_step_batch_with(cpu, bus, observers, config, max_count, |c, b, o, cfg| {
        c.step(b, o, cfg)
    })
}

/// [`default_step_batch`] with the per-instruction step supplied by the caller.
///
/// Xtensa hands in `step_body`, which is `#[inline(always)]`, so the batch
/// loop contains the instruction path and pays the call once per batch.
/// `Cpu::step` stays an ordinary call for the ticks between windows.
/// `inline(always)` for the same measured reason as [`default_step_batch`]:
/// an outlined copy of this loop moved nRF and EFR32 step-mode code that
/// never calls it.
#[inline(always)]
pub(crate) fn default_step_batch_with<C, F>(
    cpu: &mut C,
    bus: &mut dyn Bus,
    observers: &[Arc<dyn SimulationObserver>],
    config: &SimulationConfig,
    max_count: u32,
    mut step: F,
) -> SimResult<u32>
where
    C: Cpu + ?Sized,
    F: FnMut(
        &mut C,
        &mut dyn Bus,
        &[Arc<dyn SimulationObserver>],
        &SimulationConfig,
    ) -> SimResult<()>,
{
    // While push-mode logic capture is armed, the tap clock must advance
    // once per retired instruction so pad writes stamp with the cycle
    // boundary they become observable at (see `crate::logic_capture`).
    // One Arc clone + flag check per batch when idle; a relaxed atomic
    // increment per instruction while armed.
    let tap = bus.logic_tap().filter(|t| t.push_armed());
    // Issue #842: republish the live cycle per retired instruction so a
    // lazily-advanced peripheral is not pinned to the batch-start cycle for
    // the whole window. Same gate and same rationale as the hand-written
    // `CortexM::step_batch` / `RiscV::step_batch` twins — see either.
    // An armed batch stop (`Bus::batch_stop_armed`) runs at quantum 1 with
    // the per-cycle tick and drain deferred to its end, so the bus clock must
    // stay live per instruction as at a larger interval.
    let stop_armed = bus.batch_stop_armed();
    let live_step = u64::from(config.peripheral_tick_interval > 1 || stop_armed);
    // A core whose step cycles are clock time (AVR) retires instructions of
    // different lengths, so "one bump per instruction" would drift from the
    // cycle count the poll reference samples at. For that core the clock is
    // derived from the CPU's own cycle counter and events pushed by a
    // multi-cycle instruction are moved to the boundary after it.
    let timed_tap = tap
        .as_ref()
        .filter(|_| cpu.instruction_cycles_are_time())
        .map(|t| (t.clock(), cpu.clock_cycles()));
    for i in 0..max_count {
        let mut pending_before = 0;
        let mut provisional = 0;
        if let Some(tap) = &tap {
            if let Some((start, c0)) = timed_tap {
                provisional = start + cpu.clock_cycles().saturating_sub(c0) + 1;
                tap.set_clock(provisional);
                // The first instruction also owns pushes made while paused,
                // which carry the same provisional stamp.
                pending_before = if i == 0 { 0 } else { tap.pending_len() };
            } else {
                tap.bump_clock();
            }
        }
        step(cpu, bus, observers, config)?;
        if let (Some(tap), Some((start, c0))) = (&tap, timed_tap) {
            let post = start + cpu.clock_cycles().saturating_sub(c0);
            if tap.pending_len() > pending_before {
                tap.restamp_pending_from(pending_before, provisional, post);
            }
            // Leave the clock at the boundary actually reached, so a follow-up
            // `step_batch` (tick-window fill) starts from the true cycle.
            tap.set_clock(post);
        }
        // Advance after the step — see `CortexM::step_batch`.
        advance_batch_cycle(bus, live_step);
        if config.idle_fast_forward_enabled
            && (cpu.idle_fast_forward_budget(bus).is_some() || cpu.poll_loop_anchor_ready())
        {
            return Ok(i + 1);
        }
        if stop_armed && bus.batch_stop_hit() {
            return Ok(i + 1);
        }
        // A world's net pad changed drive: end the batch on this instruction
        // (`LogicTap::set_batch_stop_channels`).
        if tap.as_ref().is_some_and(|t| t.batch_stop_hit()) {
            return Ok(i + 1);
        }
    }
    Ok(max_count)
}

/// Publish cycles retired inside a CPU batch when the event scheduler is on.
///
/// Keeping the feature split at this one choke point avoids duplicating it in
/// every CPU that can retire more than one instruction at a time. The helper
/// is always inlined, so featureless builds erase the call and scheduler
/// builds retain the same `Bus::advance_cycle` dispatch as before.
#[inline(always)]
pub(crate) fn advance_batch_cycle(_bus: &mut dyn Bus, _delta: u64) {
    #[cfg(feature = "event-scheduler")]
    _bus.advance_cycle(_delta);
}

pub trait Cpu: Send {
    fn reset(&mut self, bus: &mut dyn Bus) -> SimResult<()>;
    /// JIT engine counters for this CPU, if it ran a JIT that was created.
    /// Default `None` (interpreter-only CPUs / non-JIT builds). Lets generic
    /// `C: Cpu` callers — e.g. the CLI oracle loop — assert the compiled path
    /// was non-vacuously exercised without downcasting to the concrete CPU.
    fn jit_engine_stats(&self) -> Option<CpuJitStats> {
        None
    }
    /// Set the vector table base (Cortex-M VTOR). Returns `false`, having
    /// changed nothing, on a core without one. The SoftDevice HLE
    /// (`crate::sd_hle`) points it at the application base, the forwarding
    /// the absent MBR/SoftDevice would do. A capability rather than an
    /// `as_any_mut()` + `downcast_mut::<CortexM>()` reach.
    fn set_vector_table_base(&mut self, _base: u32) -> bool {
        false
    }
    /// Address of the stacked exception frame when this core is executing
    /// its SVCall handler (exception 11) at `handler` (Thumb bit ignored):
    /// the stack the SVC was issued from, MSP or PSP per EXC_RETURN bit 2.
    /// `None` otherwise, and on every core without SVCall (the default). The
    /// SoftDevice HLE serves the call from that frame (r0-r3, stacked PC).
    fn svcall_frame_at(&self, _handler: u32) -> Option<u32> {
        None
    }
    /// Leave the current exception as if its handler had executed `bx lr`
    /// (exception return through EXC_RETURN), so an emulated handler never
    /// runs guest code. `Ok(false)` when the core has no such notion (the
    /// default).
    fn hle_return_from_exception(&mut self, _bus: &mut dyn Bus) -> SimResult<bool> {
        Ok(false)
    }
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
    /// Retire one instruction, or report zero while architecturally waiting.
    /// Existing MCU cores retain their one-instruction step contract.
    fn step_retired(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<u32> {
        self.step(bus, observers, config)?;
        Ok(1)
    }
    fn step_batch(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
        max_count: u32,
    ) -> SimResult<u32> {
        default_step_batch(self, bus, observers, config, max_count)
    }
    fn set_pc(&mut self, val: u32);
    fn get_pc(&self) -> u32;
    fn set_sp(&mut self, val: u32);
    fn set_exception_pending(&mut self, exception_num: u32);

    // Debug Access
    fn get_register(&self, id: u8) -> u32;
    fn set_register(&mut self, id: u8, val: u32);
    /// Forget every decoded or fetched instruction this core holds.
    ///
    /// A debugger write goes straight to the bus, past the core, so a core that
    /// caches decodes (or a fetch window, or JIT blocks) would go on executing
    /// what the memory USED to hold. Hosts that write memory on the user's
    /// behalf call this afterwards. Never called by a firmware run, so it
    /// cannot change what firmware does; a core with no such cache keeps the
    /// default.
    fn invalidate_code_caches(&mut self) {}
    fn snapshot(&self) -> snapshot::CpuSnapshot;
    fn apply_snapshot(&mut self, snapshot: &snapshot::CpuSnapshot);

    /// Application cores start a loaded ELF at e_entry, without the Cortex-M
    /// vector-table interpretation used by the legacy load path.
    fn uses_elf_entry_point(&self) -> bool {
        false
    }

    /// Fallible restore entry point for machines. Cores with diagnostic-only
    /// snapshots must override this rather than report a successful resume.
    fn try_apply_snapshot(&mut self, snapshot: &snapshot::CpuSnapshot) -> SimResult<()> {
        self.apply_snapshot(snapshot);
        Ok(())
    }

    /// Full mid-flight CPU state for binary runtime snapshots: the arch tag
    /// plus an opaque blob the matching CPU type knows how to parse, or
    /// `None` when this core models no runtime snapshot at all.
    ///
    /// `None` is the ONLY way to say "I cannot" — the return type carries no
    /// arch tag to fabricate. That is the point. This method used to return
    /// `(CpuKind::ArmCortexM, Vec::new())` for every core without an
    /// implementation, so an AVR reported itself as a Cortex-M and a Cortex-M
    /// reported a well-formed snapshot with an EMPTY body. Nothing downstream
    /// could tell that apart from a real capture: `snapshot capture` wrote the
    /// file, the browser handed the bytes to JS, and the resume restored no
    /// registers while every layer reported success. A simulator sold as a
    /// hardware oracle may decline to answer; it may not invent one.
    ///
    /// The default is deliberately a value and not a panic: in wasm a Rust
    /// panic lowers to an `unreachable` TRAP, which runs no destructors, so
    /// wasm-bindgen's borrow guard leaks and the `WasmSimulator` stays
    /// borrowed forever — every later call, `step_batch` included, then fails
    /// with "recursive use of an object". One snapshot attempt on an
    /// unsupported CPU bricked the whole engine, which is how Cortex-M labs
    /// died seconds into a run. `None` returns normally and leaves the module
    /// healthy.
    ///
    /// A core that overrides this MUST also override
    /// [`Self::apply_runtime_snapshot`] — a capture nothing can restore is
    /// the same lie in the other direction. `cpu_runtime_snapshot_honesty`
    /// fails the build if the two halves ever come apart.
    fn runtime_snapshot(&self) -> Option<(runtime_snapshot::CpuKind, Vec<u8>)> {
        None
    }

    /// Apply a previously-taken runtime snapshot.
    ///
    /// The default REFUSES. It used to be `Ok(())` with the bytes dropped on
    /// the floor, which made "restored" and "silently ignored" the same
    /// observable outcome: `WasmSimulator::apply_runtime_snapshot` returned
    /// success to JS after leaving the CPU cold and the peripherals warm — a
    /// machine that never existed on silicon, reported as a good resume. A
    /// core that models no restore says so, and the caller decides.
    fn apply_runtime_snapshot(
        &mut self,
        kind: runtime_snapshot::CpuKind,
        _bytes: &[u8],
    ) -> SimResult<()> {
        Err(SimulationError::NotImplemented(format!(
            "apply_runtime_snapshot: this CPU models no runtime snapshot, so a \
             {kind:?} blob cannot be restored onto it"
        )))
    }
    fn get_register_names(&self) -> Vec<String>;
    fn index_of_register(&self, name: &str) -> Option<u8>;

    /// The fault this CPU recorded, if any: the SCB fault status registers, the
    /// first fault-handler entry since reset (stacked frame + EXC_RETURN) and a
    /// LOCKUP record. `None` when nothing faulted or the core models no ARM
    /// fault registers. [`fault_verdict::decode_fault`] turns it into the
    /// one-sentence verdict.
    fn fault_capture(&self) -> Option<fault_verdict::FaultCapture> {
        None
    }

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

    /// Return the maximum number of idle cycles this CPU may skip now, or
    /// `None` when it is not architecturally waiting or an interrupt is already
    /// observable. Machine-level guards decide whether the bus/peripheral side
    /// is also safe to skip.
    fn idle_fast_forward_budget(&self, _bus: &dyn Bus) -> Option<u64> {
        None
    }

    /// Advance CPU-local time/counters for cycles skipped while idle. The
    /// default is a no-op for CPUs that do not opt into idle fast-forwarding.
    fn fast_forward_idle_cycles(&mut self, _cycles: u64) {}

    /// Cycles until a core-internal timer edge (Xtensa CCOMPARE0, the
    /// FreeRTOS tick source) will raise a wake interrupt on this
    /// WAITI-parked core, or `None` when no such edge is armed. The planner
    /// clamps a coalesced dual-idle window to this so the parked core's
    /// timer interrupt lands at the exact cycle a per-instruction step would
    /// raise it, rather than at the end of the window `fast_forward_idle_cycles`
    /// skipped over. Default `None`: cores without a CCOUNT-style timer need
    /// no clamp.
    fn parked_wake_deadline_cycles(&self) -> Option<u64> {
        None
    }

    /// True right after this core read its cycle counter at the head of a
    /// loop that came round to the same read at least twice: a candidate
    /// time-poll busy-wait (`while (ccount() - t0 < n) {}`). Batch loops end
    /// on it, as they end on an idle budget, so the machine can ask
    /// [`Self::poll_loop_skip_budget`]. Default `false`.
    #[inline]
    fn poll_loop_anchor_ready(&self) -> bool {
        false
    }

    /// Prove the loop at the current anchor is a pure function of the cycle
    /// counter and return `(period, cycles)`: one iteration's length and the
    /// most cycles (a multiple of it, at most `limit`) the machine may skip
    /// with [`Self::fast_forward_poll_loop`] and land exactly where running
    /// them would. `None` (the default) refuses and changes nothing. A busy
    /// core with such a budget counts as idle for the machine's skip.
    fn poll_loop_skip_budget(&mut self, _bus: &dyn Bus, _limit: u64) -> Option<(u64, u64)> {
        None
    }

    /// Skip `cycles` of the loop the last [`Self::poll_loop_skip_budget`]
    /// proved (a multiple of its period, at most its cycles). `false`, having
    /// changed nothing, when there is no such plan.
    fn fast_forward_poll_loop(&mut self, _cycles: u64) -> bool {
        false
    }

    /// While the machine defers this core's proven poll loop (the plan the
    /// last [`Self::poll_loop_skip_budget`] proved, not yet applied): has
    /// anything the deferred iterations depend on changed? True when a word
    /// the loop loads no longer holds the value the proof read, or when an
    /// interrupt the core would take is now pending. Default `true`: a core
    /// that cannot defer never stays deferred.
    fn poll_loop_defer_changed(&self, _bus: &dyn Bus) -> bool {
        true
    }

    /// The byte ranges `[lo, hi)` the deferred loop's proof read; a store
    /// into one may change it. Default: none.
    fn poll_loop_watch(&self, _out: &mut Vec<(u64, u64)>) {}

    /// End a deferral: leave the core exactly where running `cycles` cycles
    /// of the proven loop from its anchor would leave it, with the memory and
    /// interrupt state the proof saw. `cycles` is at most the proven budget.
    /// `false`, having changed nothing, when there is no plan.
    fn poll_loop_catch_up(&mut self, _bus: &dyn Bus, _cycles: u64) -> bool {
        false
    }

    /// Is the cycle count this core charges per step real clock time?
    ///
    /// True for an AVR: its step takes the datasheet's 1–4 clock cycles, and
    /// the core's own timers (`millis()` is Timer0) already count exactly
    /// those. A machine running such a core advances `total_cycles` by
    /// [`Self::clock_cycles`] instead of by one per instruction, so the test
    /// triggers, co-simulation and traces run on the clock the firmware sees.
    ///
    /// False (the default) where the per-step number is not timing: Cortex-M
    /// and RISC-V report instruction length there, and those machines keep
    /// one cycle per instruction.
    #[inline]
    fn instruction_cycles_are_time(&self) -> bool {
        false
    }

    /// The core's running count of clock cycles, for a core whose
    /// [`Self::instruction_cycles_are_time`] is true. The machine reads it
    /// before and after each CPU window, never per instruction.
    #[inline]
    fn clock_cycles(&self) -> u64 {
        0
    }

    /// The most clock cycles one step of this core can take (an AVR `CALL`,
    /// `RET` or interrupt entry is 4). The machine divides cycle budgets by it
    /// when planning a window, so a batch of instructions cannot run more than
    /// one step past a cycle limit or tick boundary.
    #[inline]
    fn max_step_cycles(&self) -> u32 {
        1
    }

    /// True while this core is parked in an architectural wait (e.g. Xtensa
    /// `WAITI`) and will only retire work when an interrupt wakes it.
    ///
    /// Dual-core machines use this to batch the primary core while a secondary
    /// APP CPU sits in FreeRTOS idle: lockstep quantum-1 is only required when
    /// the secondary is actively executing or still held in reset (`halted`).
    fn is_parked_idle(&self) -> bool {
        false
    }

    /// Classify a secondary core with one query on the hot planning path.
    /// Reset-held and architecturally parked are distinct: the former retires
    /// nothing, while the latter owns live counters and wake deadlines.
    fn secondary_execution_state(&self) -> SecondaryExecutionState {
        if self.is_parked_idle() {
            SecondaryExecutionState::ParkedIdle
        } else {
            SecondaryExecutionState::Active
        }
    }

    /// Phase 3.2 JIT pilot (issue #124): total number of times any
    /// JIT-compiled block on this CPU has been invoked. Default 0 for
    /// CPUs/builds without JIT support so callers can unconditionally
    /// query it in reporting paths.
    fn jit_hit_count(&self) -> u64 {
        0
    }

    /// Firmware exit code latched by the core (Cortex-M `SYS_EXIT`), taken once.
    /// Default `None`: cores other than Cortex-M have no semihosting trap.
    /// Polled by `Machine::advance` beside `drain_simctl_exit_code`. Not the
    /// simctl device — that peripheral is not on every bus.
    fn take_firmware_exit(&mut self) -> Option<u32> {
        None
    }

    /// `true` only for a core that can retire `bkpt #0xAB` as semihosting.
    /// `semihosting_contains` fails closed when this is false.
    fn supports_semihosting(&self) -> bool {
        false
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
    fn step_retired(
        &mut self,
        bus: &mut dyn Bus,
        observers: &[Arc<dyn SimulationObserver>],
        config: &SimulationConfig,
    ) -> SimResult<u32> {
        (**self).step_retired(bus, observers, config)
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
    // Forwarded. The trait defaults are no-ops, and the browser machine is
    // `Box<dyn Cpu>`, so a default here would never reach the Cortex-M impl.
    // `invalidate_code_caches` is what
    // `debug_writes::a_write_into_code_takes_effect_on_the_next_execution`
    // caught; the three SoftDevice hooks are the same trap for SVC attach.
    fn invalidate_code_caches(&mut self) {
        (**self).invalidate_code_caches()
    }
    fn set_vector_table_base(&mut self, base: u32) -> bool {
        (**self).set_vector_table_base(base)
    }
    fn svcall_frame_at(&self, handler: u32) -> Option<u32> {
        (**self).svcall_frame_at(handler)
    }
    fn hle_return_from_exception(&mut self, bus: &mut dyn Bus) -> SimResult<bool> {
        (**self).hle_return_from_exception(bus)
    }
    fn snapshot(&self) -> snapshot::CpuSnapshot {
        (**self).snapshot()
    }
    fn apply_snapshot(&mut self, s: &snapshot::CpuSnapshot) {
        (**self).apply_snapshot(s)
    }
    fn uses_elf_entry_point(&self) -> bool {
        (**self).uses_elf_entry_point()
    }
    fn try_apply_snapshot(&mut self, s: &snapshot::CpuSnapshot) -> SimResult<()> {
        (**self).try_apply_snapshot(s)
    }
    fn runtime_snapshot(&self) -> Option<(runtime_snapshot::CpuKind, Vec<u8>)> {
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
    fn fault_capture(&self) -> Option<fault_verdict::FaultCapture> {
        (**self).fault_capture()
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
    fn idle_fast_forward_budget(&self, bus: &dyn Bus) -> Option<u64> {
        (**self).idle_fast_forward_budget(bus)
    }
    fn fast_forward_idle_cycles(&mut self, cycles: u64) {
        (**self).fast_forward_idle_cycles(cycles)
    }
    fn parked_wake_deadline_cycles(&self) -> Option<u64> {
        (**self).parked_wake_deadline_cycles()
    }
    fn instruction_cycles_are_time(&self) -> bool {
        (**self).instruction_cycles_are_time()
    }
    fn clock_cycles(&self) -> u64 {
        (**self).clock_cycles()
    }
    fn max_step_cycles(&self) -> u32 {
        (**self).max_step_cycles()
    }
    fn is_parked_idle(&self) -> bool {
        (**self).is_parked_idle()
    }
    fn poll_loop_anchor_ready(&self) -> bool {
        (**self).poll_loop_anchor_ready()
    }
    fn poll_loop_skip_budget(&mut self, bus: &dyn Bus, limit: u64) -> Option<(u64, u64)> {
        (**self).poll_loop_skip_budget(bus, limit)
    }
    fn fast_forward_poll_loop(&mut self, cycles: u64) -> bool {
        (**self).fast_forward_poll_loop(cycles)
    }
    fn poll_loop_defer_changed(&self, bus: &dyn Bus) -> bool {
        (**self).poll_loop_defer_changed(bus)
    }
    fn poll_loop_watch(&self, out: &mut Vec<(u64, u64)>) {
        (**self).poll_loop_watch(out)
    }
    fn poll_loop_catch_up(&mut self, bus: &dyn Bus, cycles: u64) -> bool {
        (**self).poll_loop_catch_up(bus, cycles)
    }
    fn secondary_execution_state(&self) -> SecondaryExecutionState {
        (**self).secondary_execution_state()
    }
    fn jit_hit_count(&self) -> u64 {
        (**self).jit_hit_count()
    }
    fn take_firmware_exit(&mut self) -> Option<u32> {
        (**self).take_firmware_exit()
    }
    fn supports_semihosting(&self) -> bool {
        (**self).supports_semihosting()
    }
}
