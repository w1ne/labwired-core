//! Owns the authoritative advance loop and stop/report accounting.

use crate::{AdvanceReport, AdvanceRequest, AdvanceStop, Cpu, Machine, SimResult, SimulationError};

impl<C: Cpu> Machine<C> {
    pub fn advance(&mut self, request: AdvanceRequest) -> SimResult<AdvanceReport> {
        // Task 3 temporarily supports only the exact policy set produced by
        // `single()`. Equality here validates that public contract; it must not
        // be reused to infer boundary timing. Task 4 removes this guard when it
        // implements run requests and policy overrides.
        if request != AdvanceRequest::single() {
            return Err(SimulationError::NotImplemented(
                "Machine::advance currently supports only exact single-step requests".to_string(),
            ));
        }

        let start_cycles = self.total_cycles;
        self.total_cycles += 1;
        // Mirror the cycle count into the bus before the CPU executes, so
        // tick-time services can read "now": scheduler-driven peripheral sync
        // (event-scheduler) and the HC-SR04 echo-window timing (always). O(1) —
        // a field write + a relaxed atomic store (the shared read-sync clock),
        // not the per-peripheral walk this phase removed.
        self.bus.set_current_cycle(self.total_cycles);
        self.bus.bus_trace.set_cycle(self.total_cycles);
        // The cycle boundary this instruction's effects become observable at —
        // pad writes pushed through the logic tap stamp with it (single-step
        // path: one instruction, no CPU-side clock bumps needed).
        let logic_boundary = self.total_cycles;
        if self.logic_capture.push_active() {
            self.bus.logic_tap.set_clock(logic_boundary);
        }

        let progress = self.execute_cpu_window(1)?;
        self.record_cpu_progress(progress.primary_steps);
        self.commit_advance_boundary(logic_boundary)?;

        Ok(AdvanceReport {
            stop: AdvanceStop::FuelLimit,
            fuel_consumed: 1,
            primary_steps: u64::from(progress.primary_steps),
            secondary_steps: u64::from(progress.secondary_steps),
            elapsed_cycles: self.total_cycles - start_cycles,
            idle_cycles: 0,
            cpu_batches: 1,
        })
    }
}
