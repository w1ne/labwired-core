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
        let until_tick = tick_interval - (self.total_cycles % tick_interval);
        let mut count = until_tick.min(u64::from(u32::MAX));

        if let Some(limit) = request.limits().fuel {
            count = count.min(limit.saturating_sub(fuel_consumed));
        }
        if let Some(limit) = request.limits().simulated_cycles {
            count = count.min(limit.saturating_sub(elapsed_cycles));
        }
        if let BatchPolicy::AtMost(cap) = request.batch_policy() {
            count = count.min(u64::from(cap.get()));
        }

        // Any instruction may request an RTC or SCB reset. Commit it before
        // the next instruction, intentionally trading Cortex/RTC throughput
        // for reset fidelity even while no request is currently latched.
        let reset_fidelity = self.rtc_cntl_index.is_some() || self.scb_index.is_some();
        // Interleave both cores one scheduling quantum at a time for lockstep
        // fairness over their shared bus.
        let secondary_lockstep = self.cpu_secondary.is_some();
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
        }

        #[cfg(feature = "event-scheduler")]
        if count > 1 {
            if let Some(deadline) = self.bus.next_hcsr04_deadline_cycle() {
                let until = deadline.saturating_sub(self.total_cycles);
                count = count.min(until.clamp(1, u64::from(u32::MAX)));
            }
            if tick_interval > 1 && count > 1 {
                self.refresh_generation_scratch();
                if let Some(deadline) = self.sched.next_event_deadline(&self.generation_scratch) {
                    let until = if deadline > self.total_cycles {
                        deadline - self.total_cycles
                    } else {
                        1
                    };
                    count = count.min(until);
                }
            }
        }

        count as u32
    }
}
