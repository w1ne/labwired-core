//! Owns the authoritative advance loop and stop/report accounting.

use super::boundary::ExecutionMode;
use crate::{
    AdvanceReport, AdvanceRequest, AdvanceStop, BreakpointPolicy, Cpu, IdlePolicy, Machine,
    SimResult,
};

#[derive(Default)]
struct AdvanceState {
    fuel_consumed: u64,
    primary_steps: u64,
    secondary_steps: u64,
    idle_cycles: u64,
    cpu_batches: u64,
}

impl AdvanceState {
    fn report(&self, stop: AdvanceStop, elapsed_cycles: u64) -> AdvanceReport {
        AdvanceReport::new(
            stop,
            self.fuel_consumed,
            self.primary_steps,
            self.secondary_steps,
            elapsed_cycles,
            self.idle_cycles,
            self.cpu_batches,
        )
    }
}

impl<C: Cpu> Machine<C> {
    /// Advances the machine through its authoritative execution path.
    ///
    /// Normal stop conditions are checked before the next unit of work, in
    /// this order: honored breakpoint, fuel limit, then simulated-cycle limit.
    /// Fuel counts primary scheduling quanta plus cycles skipped by idle fast
    /// forward. A simulated-cycle limit is observed only at committed machine
    /// boundaries: CPU work is planned not to exceed the remaining budget, but
    /// an atomic boundary may charge peripheral costs and therefore report an
    /// `elapsed_cycles` value beyond that limit.
    ///
    /// On a normal stop, the returned [`AdvanceReport`] accounts for all
    /// successfully committed primary and secondary steps, idle cycles, CPU
    /// batches, fuel, and elapsed machine cycles. A CPU batch that returns
    /// `Ok(0)` stops with [`AdvanceStop::NoProgress`]. CPU errors instead return
    /// `Err` without rollback; according to the [`Cpu`] contract, the CPU may
    /// already have retired part of a batch, and direct execution may already
    /// have published its boundary clocks.
    ///
    /// A request with no fuel or simulated-cycle limit can run indefinitely.
    /// Callers must arrange an honored breakpoint, CPU progress termination,
    /// or external termination when issuing such a request.
    pub fn advance(&mut self, request: AdvanceRequest) -> SimResult<AdvanceReport> {
        let start_cycles = self.total_cycles;
        let mut state = AdvanceState::default();

        loop {
            let elapsed = self.total_cycles - start_cycles;

            if request.breakpoint_policy() == BreakpointPolicy::Honor {
                let pc = self.cpu.get_pc();
                let aligned = pc & !1;
                if self.breakpoints.contains(&aligned) && self.last_breakpoint != Some(aligned) {
                    self.last_breakpoint = Some(aligned);
                    return Ok(state.report(AdvanceStop::Breakpoint(pc), elapsed));
                }
                self.last_breakpoint = None;
            }

            if request
                .limits()
                .fuel
                .is_some_and(|limit| state.fuel_consumed >= limit)
            {
                return Ok(state.report(AdvanceStop::FuelLimit, elapsed));
            }
            if request
                .limits()
                .simulated_cycles
                .is_some_and(|limit| elapsed >= limit)
            {
                return Ok(state.report(AdvanceStop::CycleLimit, elapsed));
            }

            if request.idle_policy() == IdlePolicy::Configured {
                let fuel_remaining = request
                    .limits()
                    .fuel
                    .map(|limit| limit.saturating_sub(state.fuel_consumed));
                let cycle_remaining = request
                    .limits()
                    .simulated_cycles
                    .map(|limit| limit.saturating_sub(elapsed));
                let skip_limit = match (fuel_remaining, cycle_remaining) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                let skipped = self.try_idle_fast_forward(
                    skip_limit,
                    0,
                    request.breakpoint_policy() == BreakpointPolicy::Honor
                        && !self.breakpoints.is_empty(),
                );
                if skipped > 0 {
                    state.fuel_consumed += skipped;
                    state.idle_cycles += skipped;
                    self.logic_observe(self.total_cycles);
                    continue;
                }
            }

            self.bus.reset_mmio_activity_counters();
            let count = self.plan_cpu_window(request, state.fuel_consumed, elapsed);
            debug_assert!(count > 0);
            // Dual-core lockstep only while the secondary is active or still
            // held in reset. When APP is WAITI-parked, batch the primary.
            let secondary_active = match self.cpu_secondary.as_ref() {
                Some(sec) if sec.is_parked_idle() => false,
                Some(_) => true,
                None => false,
            };
            let mode = if request.is_single() {
                ExecutionMode::SingleDirect
            } else if secondary_active {
                ExecutionMode::RunDual
            } else {
                ExecutionMode::RunBatch
            };
            let batch_start = self.total_cycles;
            let progress = self.execute_cpu_window(mode, count)?;
            if progress.primary_steps == 0 {
                return Ok(state.report(AdvanceStop::NoProgress, self.total_cycles - start_cycles));
            }

            self.commit_advance_boundary(mode, batch_start, progress)?;
            state.fuel_consumed += u64::from(progress.primary_steps);
            state.primary_steps += u64::from(progress.primary_steps);
            state.secondary_steps += u64::from(progress.secondary_steps);
            state.cpu_batches += 1;
        }
    }
}
