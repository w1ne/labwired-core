//! Owns the authoritative advance loop and stop/report accounting.

use super::boundary::{CoreProgress, ExecutionMode};
use crate::{
    AdvanceReport, AdvanceRequest, AdvanceStop, BreakpointPolicy, Cpu, HostTimeMode, IdlePolicy,
    Machine, SimResult, SimulationConfig, SimulationObserver,
};
use std::sync::Arc;
use std::time::Duration;

/// Executes one planned CPU window for
/// [`Machine::advance_with_window_runner`].
///
/// `count` is the width [`Machine`] planned for this window; the runner
/// retires at most that many instructions and reports how many it did. It is
/// handed the CPU, bus, observers and config so an out-of-tree backend (the
/// browser JIT) can dispatch into its own compiled blocks and fall back to
/// [`Cpu::step`]. Everything a window runner must **not** reimplement — tick
/// boundary clamping, peripheral tick cadence, reset drains, idle fast
/// forward, work/stop accounting — stays in the advance loop that calls it.
pub type WindowRunner<'a, C> = dyn FnMut(
        &mut C,
        &mut crate::bus::SystemBus,
        &[Arc<dyn SimulationObserver>],
        &SimulationConfig,
        u32,
    ) -> SimResult<u32>
    + 'a;

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
    fn pace_realtime(&self, start_cycles: u64, start_wall: Duration) {
        if self.config.host_time_mode != HostTimeMode::Realtime {
            return;
        }
        crate::host_time::pace(
            self.config.host_time_mode,
            self.bus.cpu_hz,
            start_cycles,
            self.total_cycles,
            start_wall,
            self.host_clock.now(),
            self.host_clock.as_ref(),
        );
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
        self.advance_inner(request, None)
    }

    /// Advances the machine exactly like [`Self::advance`], but executes each
    /// planned CPU window through `run_window` instead of the in-tree CPU
    /// path.
    ///
    /// This is the seam for a CPU backend that lives outside `labwired-core`
    /// — the browser's Thumb JIT — without opening a second dispatcher: the
    /// window plan, the boundary commit (peripheral tick cadence, scheduler
    /// drains, reset latches, logic capture), idle fast forward, breakpoints,
    /// and stop/report accounting are all the authoritative ones.
    ///
    /// The runner is only consulted for single-core `RunBatch` windows. A
    /// dual-core machine (which must lockstep two CPUs) and `AdvanceRequest::single`
    /// windows fall back to the in-tree path, so a runner can never silently
    /// skip a secondary core or a single-step contract. `run_window` must
    /// retire at most the `count` it is given; a larger report is clamped to
    /// the plan.
    pub fn advance_with_window_runner<F>(
        &mut self,
        request: AdvanceRequest,
        mut run_window: F,
    ) -> SimResult<AdvanceReport>
    where
        F: FnMut(
            &mut C,
            &mut crate::bus::SystemBus,
            &[Arc<dyn SimulationObserver>],
            &SimulationConfig,
            u32,
        ) -> SimResult<u32>,
    {
        self.advance_inner(request, Some(&mut run_window))
    }

    fn advance_inner(
        &mut self,
        request: AdvanceRequest,
        mut run_window: Option<&mut WindowRunner<'_, C>>,
    ) -> SimResult<AdvanceReport> {
        let start_cycles = self.total_cycles;
        // Read the wall origin only when something will consume it:
        // `pace_realtime` returns immediately outside `Realtime`, and the CLI's
        // single-step loop issues one `advance` per simulated instruction, so an
        // unconditional `Instant::elapsed()` here is a per-instruction cost —
        // 91 Ir/step under callgrind, +8.5% on every ARM board that no engine
        // change earned.
        let start_wall = if self.config.host_time_mode == HostTimeMode::Realtime {
            self.host_clock.now()
        } else {
            Duration::ZERO
        };
        let mut state = AdvanceState::default();

        loop {
            let elapsed = self.total_cycles - start_cycles;

            // Release a dual-core ESP32-S3's APP_CPU on the real hardware edge:
            // the PRO_CPU clearing `SYSTEM_CORE_1_CONTROL_0.RESETING`, surfaced
            // by the SYSTEM peripheral as `APPCPU_RESET_RELEASED`.
            //
            // This belongs here rather than in a frontend because *every*
            // consumer needs it. It used to live only in the native runner's
            // step loop, so a dual-core ESP-IDF image booted natively and
            // stalled forever at `cpu_start: Multicore app` in the browser —
            // core 1 was constructed but never let out of reset. Taking the
            // flag here is harmless for a frontend that also checks it (the
            // first taker wins and both unhalt the same core) and a no-op on
            // every chip that never sets it.
            //
            // Not, however, when the secondary has no ROM to boot. A fast-boot
            // frontend swaps the mask ROM for a thunk harness, so the reset
            // vector core 1 is constructed on holds no startup code: releasing
            // it there runs the harness and faults as `cause=0 at pc=0x0` a few
            // hundred steps in. Such frontends set `secondary_awaits_boot_addr`
            // and hand core 1 over at `call_start_cpu1` (`APPCPU_BOOT_ADDR`)
            // instead, which is what `release_secondary_cpu_if_requested` acts
            // on. Drain the flag either way so it cannot fire later.
            if crate::peripherals::esp_xtensa_common::rom_thunks::APPCPU_RESET_RELEASED
                .with(|s| s.take())
                && !self.secondary_awaits_boot_addr
            {
                if let Some(cpu1) = self.cpu_secondary.as_mut() {
                    cpu1.unhalt();
                }
            }

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
                    self.pace_realtime(start_cycles, start_wall);
                    continue;
                }
            }

            self.bus.reset_mmio_activity_counters();
            let count = self.plan_cpu_window(request, state.fuel_consumed, elapsed);
            debug_assert!(count > 0);
            // Dual-core lockstep only while the secondary is active or still
            // held in reset. When APP is WAITI-parked, batch the primary.
            let mode = if request.is_single() {
                ExecutionMode::SingleDirect
            } else {
                let secondary_active = self.cpu_secondary.as_ref().is_some_and(|sec| {
                    sec.secondary_execution_state() == crate::SecondaryExecutionState::Active
                });
                if secondary_active {
                    ExecutionMode::RunDual
                } else {
                    ExecutionMode::RunBatch
                }
            };
            let batch_start = self.total_cycles;
            let progress = match run_window.as_deref_mut() {
                Some(runner) if mode == ExecutionMode::RunBatch && self.cpu_secondary.is_none() => {
                    // Mirror `execute_cpu_window`'s RunBatch pre-window
                    // publication: MMIO inside the window resolves against the
                    // same clock origin the in-tree path would use.
                    self.bus.set_current_cycle(self.total_cycles);
                    self.bus.bus_trace.set_cycle(self.total_cycles);
                    if self.logic_capture.push_active() {
                        self.bus.logic_tap.set_clock(self.total_cycles);
                    }
                    let retired = runner(
                        &mut self.cpu,
                        &mut self.bus,
                        &self.observers,
                        &self.config,
                        count,
                    )?;
                    debug_assert!(
                        retired <= count,
                        "window runner retired {retired} instructions for a {count} window"
                    );
                    CoreProgress {
                        primary_steps: retired.min(count),
                        secondary_steps: 0,
                        timed_cycles: None,
                        internally_committed_cycles: false,
                    }
                }
                _ => self.execute_cpu_window(mode, count)?,
            };
            if progress.primary_steps == 0 {
                return Ok(state.report(AdvanceStop::NoProgress, self.total_cycles - start_cycles));
            }

            self.commit_advance_boundary(mode, batch_start, progress)?;

            // Firmware-authored verdict. Drained here — after the batch's
            // writes have committed — so the `EXIT` store and a semihosting
            // `SYS_EXIT` are observed with the instruction that made them
            // retired. simctl is `None` on every bus without that device.
            // The CPU hook is `None` for every core except Cortex-M, which
            // returns the latched code once. Either one stops the run. Do not
            // write the simctl device from the BKPT path: it is not on every bus.
            let simctl_exit = self.drain_simctl_exit_code();
            let cpu_exit = self.cpu.take_firmware_exit();
            if let Some(code) = simctl_exit.or(cpu_exit) {
                state.fuel_consumed += u64::from(progress.primary_steps);
                state.primary_steps += u64::from(progress.primary_steps);
                state.secondary_steps += u64::from(progress.secondary_steps);
                state.cpu_batches += 1;
                self.pace_realtime(start_cycles, start_wall);
                return Ok(state.report(
                    AdvanceStop::FirmwareExit { code },
                    self.total_cycles - start_cycles,
                ));
            }

            state.fuel_consumed += u64::from(progress.primary_steps);
            state.primary_steps += u64::from(progress.primary_steps);
            state.secondary_steps += u64::from(progress.secondary_steps);
            state.cpu_batches += 1;
            self.pace_realtime(start_cycles, start_wall);
        }
    }
}
