//! Owns safe batch-width and idle-fast-forward planning.

use crate::{AdvanceRequest, BatchPolicy, BreakpointPolicy, Bus, Cpu, Machine};

/// Narrows the planned batch width, remembering which clause did it.
///
/// Every clamp in `plan_cpu_window` goes through this so the binding clause is
/// named rather than inferred. Under the default feature set `clause` is
/// unused and this is a plain `min` — see `machine::quantum_trace`.
/// Narrow the window, and narrow the FILL CEILING with it.
///
/// `$untick` is the same window with the tick-boundary clauses left out. It is
/// how far the executor may keep going to finish filling a tick window: every
/// other clause still applies, so filling can never run past a scheduler
/// deadline, a fuel limit or a cycle limit just because the tick boundary was
/// the lowest of them.
macro_rules! clamp {
    ($count:ident, $untick:ident, $binder:ident, $clause:expr, $limit:expr) => {{
        let limit = $limit;
        if limit < $count {
            $count = limit;
            #[cfg(feature = "quantum-trace")]
            {
                $binder = $clause;
            }
        }
        if limit < $untick {
            $untick = limit;
        }
    }};
}

/// Narrow the window WITHOUT lowering the fill ceiling.
///
/// Only the tick-boundary clauses use this. The ceiling has to stay where the
/// other clauses put it, because finishing the tick window is exactly what the
/// executor is allowed to do — and nothing else.
macro_rules! clamp_tick {
    ($count:ident, $binder:ident, $clause:expr, $limit:expr) => {{
        let limit = $limit;
        if limit < $count {
            $count = limit;
            #[cfg(feature = "quantum-trace")]
            {
                $binder = $clause;
            }
        }
    }};
}

/// One planned CPU window: the instructions it may retire, and — on a core
/// whose instruction cycles are CLOCK TIME — how much further it may go to
/// finish the current peripheral tick.
///
/// Two numbers because they are not the same on every core. Where one
/// instruction is one cycle, `steps` already reaches the tick boundary and
/// `fill` is `None`. Where instruction cycles are clock time (AVR, 1..=4),
/// `steps` is the window divided by the LONGEST instruction, so a window of
/// cheaper instructions stops short of the boundary, the leftover is divided
/// again next plan, and the widths decay geometrically: measured 23.68 on
/// `atmega328p` where `512 / 4` is 128.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CpuWindow {
    pub(crate) steps: u32,
    pub(crate) fill: Option<WindowFill>,
}

/// How far a timed-cycle core may keep going to finish its tick window.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WindowFill {
    /// Cycles from the window's start to the next tick boundary.
    pub(crate) to_cycles: u64,
    /// Hard ceiling on retired instructions: the window as every clause
    /// EXCEPT the tick boundary would have sized it.
    ///
    /// This is the part the first attempt at this got wrong. `steps` is the
    /// minimum across ALL clauses, so filling past it can violate whichever
    /// clause was second-lowest — a scheduler deadline, a fuel limit — even
    /// when the tick boundary is what bound this particular window. Capping
    /// here is what makes finishing the tick the only thing the executor is
    /// allowed to do.
    ///
    /// When the tick boundary was NOT the binding clause this equals `steps`,
    /// so the executor fills nothing and the old path is preserved exactly —
    /// no feature flag, no predicate that can be inert in a shipping build.
    pub(crate) max_steps: u32,
}

impl<C: Cpu> Machine<C> {
    pub(crate) fn plan_cpu_window(
        &mut self,
        request: AdvanceRequest,
        fuel_consumed: u64,
        elapsed_cycles: u64,
    ) -> CpuWindow {
        use crate::machine::quantum_trace::clause;

        self.service_resident_edges_at_boundary();
        let tick_interval = u64::from(self.config.peripheral_tick_interval.max(1));
        // Every clamp below that is measured in cycles is turned into a step
        // count. One step is one cycle on most cores; on a core whose step
        // cycles are clock time (AVR, up to 4) the budget is divided by the
        // longest step, so a window never runs more than one step past a
        // cycle limit, a tick boundary or a deadline. `1` for every other
        // concrete core, so this compiles to the identity there.
        let step_cycles = if self.cpu.instruction_cycles_are_time() {
            u64::from(self.cpu.max_step_cycles().max(1))
        } else {
            1
        };
        let steps_within = |cycles: u64| (cycles / step_cycles).max(1);
        let mut count = u64::from(u32::MAX);
        // The same window with the tick-boundary clauses left out: the ceiling
        // the executor may fill up to. Equals `count` whenever the tick
        // boundary was not the lowest clause.
        let mut count_untick = u64::from(u32::MAX);
        #[cfg_attr(not(feature = "quantum-trace"), allow(unused_mut, unused_variables))]
        let mut binder = clause::UNBOUNDED;

        if let Some(limit) = request.limits().fuel {
            clamp!(
                count,
                count_untick,
                binder,
                clause::FUEL_LIMIT,
                limit.saturating_sub(fuel_consumed)
            );
        }
        if let Some(limit) = request.limits().simulated_cycles {
            clamp!(
                count,
                count_untick,
                binder,
                clause::CYCLE_LIMIT,
                steps_within(limit.saturating_sub(elapsed_cycles))
            );
        }
        if let BatchPolicy::AtMost(cap) = request.batch_policy() {
            clamp!(
                count,
                count_untick,
                binder,
                clause::BATCH_POLICY,
                u64::from(cap.get())
            );
        }
        if let Some(deadline) = self.bus.next_motor_service_deadline_cycle() {
            clamp!(
                count,
                count_untick,
                binder,
                clause::MOTOR_DEADLINE,
                steps_within(deadline.saturating_sub(self.total_cycles))
            );
        }

        if let Some(deadline) = self.bus.next_cpu_interrupt_deadline_cycle() {
            clamp!(
                count,
                count_untick,
                binder,
                clause::CPU_INTERRUPT_DEADLINE,
                steps_within(deadline.saturating_sub(self.total_cycles))
            );
        }

        // Dual-core: only lockstep while APP is active or still in reset-hold.
        // WAITI-parked APP (FreeRTOS idle) lets PRO batch.
        // A single-step request is already clamped to one instruction. Its
        // secondary classification cannot narrow the plan further, so avoid
        // charging that query to every reference-path instruction.
        let secondary_state = if request.is_single() {
            None
        } else {
            self.secondary_plan_state()
        };
        let secondary_parked = secondary_state == Some(crate::SecondaryExecutionState::ParkedIdle);
        let secondary_reset_held =
            secondary_state == Some(crate::SecondaryExecutionState::ResetHeld);
        let secondary_lockstep =
            self.cpu_secondary.is_some() && !secondary_parked && !secondary_reset_held;

        // Reset fidelity is enforced by the party that can see the request,
        // not by pinning the quantum for the life of the bus:
        //   * SCB (Cortex-M SYSRESETREQ) — the CPU batch loop breaks on the
        //     instruction that writes AIRCR, via the latch shared by
        //     `configure_cortex_m` (`CortexM::sysreset_signal`), so the
        //     boundary drain lands exactly where quantum-1 put it.
        //   * RTC_CNTL (ESP SW_SYS_RST) — clamps only while a request is
        //     actually latched; `boundary.rs` steps the dual-core WAITI window
        //     one instruction at a time and breaks the moment it latches.
        // `scb_index.is_some()` used to sit here too, and since
        // `configure_cortex_m` installs an SCB on EVERY Cortex-M bus that
        // meant every ARM board ran at one instruction per batch forever —
        // discarding the whole walk-deletion batching win (measured ~16x on
        // NUCLEO-L476RG, ~9x on NUCLEO-F401RE).
        //
        // The other half of that clause's old rationale — "cycle-accurate push
        // capture" — was already carried elsewhere and needs nothing here:
        // push-mode capture advances the tap clock per RETIRED INSTRUCTION
        // inside the batch (`CortexM::step_batch` calls `tap.bump_clock()`
        // before each `step_internal`), and poll-mode capture has its own
        // `poll_sampling` arm below.
        let reset_fidelity = self.rtc_cntl_reset_pending();

        // Pending cycle-accurate bus cells and operations require a lifecycle
        // commit after every instruction.
        let cycle_accurate_bus = self.bus.requires_cycle_accurate();
        // Poll-mode capture must sample every committed instruction boundary.
        let poll_sampling = self.logic_capture.poll_active();
        // Honored breakpoints must be observed before executing past their PC.
        let honored_breakpoints =
            request.breakpoint_policy() == BreakpointPolicy::Honor && !self.breakpoints.is_empty();
        // An armed debug watch must be observed before the instruction AFTER
        // the watched access retires: the latch lives on the bus and
        // `advance_inner` consumes it at the next window boundary, so the
        // window has to be the access's own instruction. Without this clamp
        // `run(Some(n))` would retire the whole n-wide window (and stop a
        // batch later) before the latch was ever read. Watches are an
        // interactive debugger facility; the throughput cost is acceptable and
        // mirrors the honored-breakpoint clause above.
        let armed_watchpoints = !self.bus.debug_watches.is_empty();

        if reset_fidelity
            || secondary_lockstep
            || cycle_accurate_bus
            || poll_sampling
            || honored_breakpoints
            || armed_watchpoints
        {
            // Attribute to the specific arm, not the disjunction: "something in
            // this `if` fired" is the answer that made #835 an elimination
            // exercise in the first place.
            #[cfg_attr(not(feature = "quantum-trace"), allow(unused_variables))]
            let arm = if reset_fidelity {
                clause::RESET_FIDELITY
            } else if secondary_lockstep {
                clause::SECONDARY_LOCKSTEP
            } else if cycle_accurate_bus {
                clause::CYCLE_ACCURATE_BUS
            } else if poll_sampling {
                clause::POLL_SAMPLING
            } else if armed_watchpoints {
                clause::HONORED_WATCHPOINTS
            } else {
                clause::HONORED_BREAKPOINTS
            };
            clamp!(count, count_untick, binder, arm, 1);
        } else if secondary_parked || secondary_reset_held {
            // Coalesced dual-core idle batch: while the secondary core is
            // WAITI-parked the primary may retire several instructions per
            // machine boundary. Commit advances peripherals once with
            // elapsed = primary_steps (see boundary.rs).
            //
            // ⚠️ It may NOT run past the next peripheral tick boundary. This
            // clause used to clamp at a flat 1024 and skip the tick clamp
            // entirely — the comment even advertised "multi-instruction PRO
            // windows even when tick_interval is 1". That is not a batching
            // decision, it is a licence to stop observing peripherals: nothing
            // between the two boundaries re-derives an interrupt level, so an
            // IRQ raised at instruction 1 of the window is not seen by the CPU
            // until instruction 1024, whatever the caller set the tick interval
            // to. ESP-IDF's SMP FreeRTOS does not survive that — see the
            // `sync_esp32s3_irq_write` write-choke note in `bus/routing.rs` for
            // the deadlock it produces (`portYIELD_WITHIN_API` lands late,
            // `xQueueReceive` re-blocks an already-blocked task, `vListInsert`
            // links the event-list item to itself and spins forever with the
            // queue spinlock held). Both halves are needed: the write choke
            // makes an MMIO-raised level visible at the write, this clamp keeps
            // a *timed* one visible within one tick interval.
            //
            // The coalescing win survives wherever it was ever sound — at
            // `tick_interval > 1` (the browser's `RECOMMENDED_TICK_INTERVAL`)
            // the window is still hundreds of instructions wide. At interval 1
            // the caller asked for per-cycle peripheral service and now gets it.
            //
            // This flat 1024 is itself further narrowed below by the
            // `next_event_deadline()` clamp — previously only applied on the
            // non-parked path — so a scheduler-driven peripheral's interrupt
            // (e.g. the ESP32-S3 SYSTIMER TARGET0 alarm) can never be
            // delivered late just because the APP core happened to be
            // WAITI-parked when it came due. See
            // `docs/performance/2026-09-18-xtensa-batched.md` for the trace
            // that caught a real ~1000-cycle-late delivery here.
            clamp!(count, count_untick, binder, clause::SECONDARY_PARKED, 1024);
            // Both end the window on the next tick boundary, sharing one clamp.
            //
            // A RESET-HELD secondary is never stepped, so `boundary.rs` does not
            // commit this window as coalesced (that needs `secondary_steps > 0`)
            // and ticks peripherals only when the window LANDS on the tick
            // grid. At tick_interval 1 its reset-held arm ticks after every
            // instruction itself; above 1 nothing does, so the window has to end
            // on the grid here -- the normal single-core path's clamp, which is
            // what a core with nothing running on its twin is. Without it a
            // 1024-wide window that starts off-grid never lands again: on the
            // ESP32-S3 TIER1 image 25 peripheral ticks ran in 10M batched
            // steps, the GDMA M2M transfer (which runs in `tick_with_bus`) never
            // happened, and `TIER1 dma` failed batched while passing stepped
            // (#68).
            // Separately, a write can arm a grid waveform inside this window:
            // stop on the next grid even when no edge was pending before the
            // batch.
            if (secondary_reset_held && tick_interval > 1) || self.bus.has_grid_gpio_schedules() {
                let until_tick = tick_interval - (self.total_cycles % tick_interval);
                clamp_tick!(
                    count,
                    binder,
                    clause::TICK_BOUNDARY,
                    steps_within(until_tick)
                );
            }
            // The parked core's own timer (Xtensa CCOMPARE0 — the FreeRTOS
            // tick source on the APP CPU) is not a scheduler event: the parked
            // path fast-forwards CCOUNT over the window and would only raise
            // the edge at its end, up to 1023 cycles after a quantum-1 run
            // raises it. End the window on the edge instead.
            // A deferred poll loop ends its deferral the same way.
            let secondary_until = match self.secondary_defer {
                Some((_, end)) => Some(end.saturating_sub(self.total_cycles).max(1)),
                None => self
                    .cpu_secondary
                    .as_ref()
                    .and_then(|sec| sec.parked_wake_deadline_cycles()),
            };
            if let Some(until) = secondary_until {
                clamp!(
                    count,
                    count_untick,
                    binder,
                    clause::SECONDARY_WAKE_DEADLINE,
                    steps_within(until)
                );
            }
        } else {
            // Normal path: batch only up to the next peripheral tick boundary.
            let until_tick = tick_interval - (self.total_cycles % tick_interval);
            clamp_tick!(
                count,
                binder,
                clause::TICK_BOUNDARY,
                steps_within(until_tick)
            );
        }

        // Scheduler/resident waveform deadlines narrow the window on BOTH the
        // non-parked (tick-boundary) path and the secondary-parked
        // (coalesced dual-idle) path above: a pending event must never be
        // delivered later than its deadline just because the window's flat
        // cap (tick boundary, or 1024 while WAITI-parked) would otherwise run
        // past it. Previously gated on `!secondary_parked`, which let a
        // scheduler-driven peripheral's interrupt (e.g. ESP32-S3 SYSTIMER
        // TARGET0) fire up to ~1023 cycles late whenever the APP core was
        // idle-parked — see `docs/performance/2026-09-18-xtensa-batched.md`.
        if count > 1 {
            if let Some(deadline) = self.bus.next_resident_edge_deadline_cycle() {
                let until = deadline.saturating_sub(self.total_cycles);
                clamp!(
                    count,
                    count_untick,
                    binder,
                    clause::RESIDENT_EDGE_DEADLINE,
                    steps_within(until).min(u64::from(u32::MAX))
                );
            }
        }
        // The scheduler heap is always present (empty without the
        // `event-scheduler` feature, where nothing enqueues), so this clamp
        // needs no feature gate of its own.
        if tick_interval > 1 && count > 1 {
            if let Some(deadline) = self.sched.next_event_deadline() {
                let until = if deadline > self.total_cycles {
                    deadline - self.total_cycles
                } else {
                    1
                };
                clamp!(
                    count,
                    count_untick,
                    binder,
                    clause::SCHEDULER_DEADLINE,
                    steps_within(until)
                );
            }
        }

        let count = count.max(1);
        #[cfg(feature = "quantum-trace")]
        crate::machine::quantum_trace::record(binder, count);
        let count_untick = count_untick.max(count);
        // No `binder` test here on purpose. The first attempt gated this on
        // `binder == TICK_BOUNDARY`, and `binder` is only assigned under
        // `quantum-trace` — so in the build that ships the condition was
        // always false and the whole thing was a no-op that still compiled,
        // passed and reported zero regressions. The ceiling carries the same
        // information without a feature: if the tick boundary was not the
        // binding clause, `count_untick == count` and the fill loop does
        // nothing.
        let fill = (step_cycles > 1 && count_untick > count).then(|| WindowFill {
            to_cycles: tick_interval - (self.total_cycles % tick_interval),
            max_steps: count_untick.min(u64::from(u32::MAX)) as u32,
        });
        CpuWindow {
            steps: count as u32,
            fill,
        }
    }
}
