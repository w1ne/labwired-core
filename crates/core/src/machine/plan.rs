//! Owns safe batch-width and idle-fast-forward planning.

use crate::{AdvanceRequest, BatchPolicy, BreakpointPolicy, Cpu, Machine};

impl<C: Cpu> Machine<C> {
    pub(crate) fn plan_cpu_window(
        &mut self,
        request: AdvanceRequest,
        fuel_consumed: u64,
        elapsed_cycles: u64,
    ) -> u32 {
        let tick_interval = u64::from(self.config.peripheral_tick_interval.max(1));
        let mut count = u64::from(u32::MAX);

        if let Some(limit) = request.limits().fuel {
            count = count.min(limit.saturating_sub(fuel_consumed));
        }
        if let Some(limit) = request.limits().simulated_cycles {
            count = count.min(limit.saturating_sub(elapsed_cycles));
        }
        if let BatchPolicy::AtMost(cap) = request.batch_policy() {
            count = count.min(u64::from(cap.get()));
        }
        if let Some(deadline) = self.bus.next_motor_service_deadline_cycle() {
            count = count.min(deadline.saturating_sub(self.total_cycles).max(1));
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
            count = count.min(1);
        } else if secondary_parked {
            // Coalesced dual-core idle batch: allow multi-instruction PRO
            // windows even when tick_interval is 1. Commit advances peripherals
            // once with elapsed = primary_steps (see boundary.rs).
            count = count.min(1024);
        } else {
            // Normal path: batch only up to the next peripheral tick boundary.
            let until_tick = tick_interval - (self.total_cycles % tick_interval);
            count = count.min(until_tick);
        }

        #[cfg(feature = "event-scheduler")]
        if count > 1 && !secondary_parked {
            if let Some(deadline) = self.bus.next_hcsr04_deadline_cycle() {
                let until = deadline.saturating_sub(self.total_cycles);
                count = count.min(until.clamp(1, u64::from(u32::MAX)));
            }
            if tick_interval > 1 && count > 1 {
                if let Some(deadline) = self.sched.next_event_deadline() {
                    let until = if deadline > self.total_cycles {
                        deadline - self.total_cycles
                    } else {
                        1
                    };
                    count = count.min(until);
                }
            }
        }

        count.max(1) as u32
    }
}
