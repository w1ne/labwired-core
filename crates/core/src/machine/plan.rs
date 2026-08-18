//! Owns safe batch-width and idle-fast-forward planning.

use crate::{AdvanceRequest, BatchPolicy, BreakpointPolicy, Cpu, Machine};

/// Narrows the planned batch width, remembering which clause did it.
///
/// Every clamp in `plan_cpu_window` goes through this so the binding clause is
/// named rather than inferred. Under the default feature set `clause` is
/// unused and this is a plain `min` — see `machine::quantum_trace`.
macro_rules! clamp {
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

impl<C: Cpu> Machine<C> {
    pub(crate) fn plan_cpu_window(
        &mut self,
        request: AdvanceRequest,
        fuel_consumed: u64,
        elapsed_cycles: u64,
    ) -> u32 {
        use crate::machine::quantum_trace::clause;

        let tick_interval = u64::from(self.config.peripheral_tick_interval.max(1));
        let mut count = u64::from(u32::MAX);
        #[cfg_attr(not(feature = "quantum-trace"), allow(unused_mut, unused_variables))]
        let mut binder = clause::UNBOUNDED;

        if let Some(limit) = request.limits().fuel {
            clamp!(
                count,
                binder,
                clause::FUEL_LIMIT,
                limit.saturating_sub(fuel_consumed)
            );
        }
        if let Some(limit) = request.limits().simulated_cycles {
            clamp!(
                count,
                binder,
                clause::CYCLE_LIMIT,
                limit.saturating_sub(elapsed_cycles)
            );
        }
        if let BatchPolicy::AtMost(cap) = request.batch_policy() {
            clamp!(count, binder, clause::BATCH_POLICY, u64::from(cap.get()));
        }
        if let Some(deadline) = self.bus.next_motor_service_deadline_cycle() {
            clamp!(
                count,
                binder,
                clause::MOTOR_DEADLINE,
                deadline.saturating_sub(self.total_cycles).max(1)
            );
        }

        // Dual-core: only lockstep while APP is active or still in reset-hold.
        // WAITI-parked APP (FreeRTOS idle) lets PRO batch.
        let secondary_parked = self
            .cpu_secondary
            .as_ref()
            .is_some_and(|sec| sec.is_parked_idle());
        let secondary_lockstep = self.cpu_secondary.is_some() && !secondary_parked;

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

        if reset_fidelity
            || secondary_lockstep
            || cycle_accurate_bus
            || poll_sampling
            || honored_breakpoints
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
            } else {
                clause::HONORED_BREAKPOINTS
            };
            clamp!(count, binder, arm, 1);
        } else if secondary_parked {
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
            clamp!(count, binder, clause::SECONDARY_PARKED, 1024);
        } else {
            // Normal path: batch only up to the next peripheral tick boundary.
            let until_tick = tick_interval - (self.total_cycles % tick_interval);
            clamp!(count, binder, clause::TICK_BOUNDARY, until_tick);
        }

        #[cfg(feature = "event-scheduler")]
        if count > 1 && !secondary_parked {
            if let Some(deadline) = self.bus.next_hcsr04_deadline_cycle() {
                let until = deadline.saturating_sub(self.total_cycles);
                clamp!(
                    count,
                    binder,
                    clause::HCSR04_DEADLINE,
                    until.clamp(1, u64::from(u32::MAX))
                );
            }
            if tick_interval > 1 && count > 1 {
                if let Some(deadline) = self.sched.next_event_deadline() {
                    let until = if deadline > self.total_cycles {
                        deadline - self.total_cycles
                    } else {
                        1
                    };
                    clamp!(count, binder, clause::SCHEDULER_DEADLINE, until);
                }
            }
        }

        let count = count.max(1);
        #[cfg(feature = "quantum-trace")]
        crate::machine::quantum_trace::record(binder, count);
        count as u32
    }
}
