// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::num::NonZeroU32;

mod advance;
mod boundary;
mod plan;

/// Controls whether an advance request observes configured breakpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointPolicy {
    /// Advance without stopping at configured breakpoints.
    Ignore,
    /// Stop when execution reaches a configured breakpoint.
    Honor,
}

/// Controls whether the machine may safely fast-forward while idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdlePolicy {
    /// Do not fast-forward idle simulated cycles.
    Disabled,
    /// Use the machine's configured idle-fast-forward behavior and safety gates.
    Configured,
}

/// Controls the maximum number of primary scheduling quanta in one CPU batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchPolicy {
    /// Let the machine select a safe batch width from active boundaries.
    Auto,
    /// Additionally cap each CPU batch at the given non-zero width.
    AtMost(NonZeroU32),
}

/// Optional budgets that terminate an advance request normally when exhausted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdvanceLimits {
    /// Maximum fuel units: one per primary scheduling quantum or idle-skipped cycle.
    ///
    /// `None` allows unbounded fuel consumption.
    pub fuel: Option<u64>,
    /// Maximum simulated machine cycles that may elapse before starting the
    /// next atomic execution boundary. Peripheral costs committed with the
    /// final boundary may make the reported elapsed time exceed this value.
    ///
    /// `None` disables the simulated-cycle budget.
    pub simulated_cycles: Option<u64>,
}

/// Private boundary-timing mode selected by the request constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvanceMode {
    /// Preserve single-step cycle-publication timing.
    Single,
    /// Use continuous-run boundary timing.
    Run,
}

/// Policies and budgets for one machine advance operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdvanceRequest {
    limits: AdvanceLimits,
    breakpoints: BreakpointPolicy,
    idle: IdlePolicy,
    batching: BatchPolicy,
    mode: AdvanceMode,
}

impl AdvanceRequest {
    /// Creates a one-quantum single-step request.
    ///
    /// Single-step ignores breakpoints, disables idle fast-forward, and limits
    /// both fuel and CPU batch width to one primary scheduling quantum.
    pub fn single() -> Self {
        Self {
            limits: AdvanceLimits {
                fuel: Some(1),
                simulated_cycles: None,
            },
            breakpoints: BreakpointPolicy::Ignore,
            idle: IdlePolicy::Disabled,
            batching: BatchPolicy::AtMost(NonZeroU32::new(1).unwrap()),
            mode: AdvanceMode::Single,
        }
    }

    /// Creates a continuous-run request with an optional fuel budget.
    ///
    /// Run requests honor breakpoints, use configured idle fast-forwarding, and
    /// select batch widths automatically. A `None` fuel value runs without a
    /// fuel limit.
    pub fn run(fuel: Option<u64>) -> Self {
        Self {
            limits: AdvanceLimits {
                fuel,
                simulated_cycles: None,
            },
            breakpoints: BreakpointPolicy::Honor,
            idle: IdlePolicy::Configured,
            batching: BatchPolicy::Auto,
            mode: AdvanceMode::Run,
        }
    }

    /// Sets the maximum number of simulated machine cycles that may elapse.
    pub fn with_cycle_limit(mut self, cycles: u64) -> Self {
        self.limits.simulated_cycles = Some(cycles);
        self
    }

    /// Caps each CPU batch without changing the request's other policies.
    pub fn with_batch_cap(mut self, cap: NonZeroU32) -> Self {
        self.batching = BatchPolicy::AtMost(cap);
        self
    }

    /// Sets whether configured breakpoints terminate this request.
    pub fn with_breakpoints(mut self, policy: BreakpointPolicy) -> Self {
        self.breakpoints = policy;
        self
    }

    /// Returns the request's fuel and simulated-cycle budgets.
    pub fn limits(self) -> AdvanceLimits {
        self.limits
    }

    /// Returns the request's breakpoint policy.
    pub fn breakpoint_policy(self) -> BreakpointPolicy {
        self.breakpoints
    }

    /// Returns the request's idle-fast-forward policy.
    pub fn idle_policy(self) -> IdlePolicy {
        self.idle
    }

    /// Returns the request's CPU batching policy.
    pub fn batch_policy(self) -> BatchPolicy {
        self.batching
    }

    /// Reports whether this request preserves single-step boundary timing.
    pub(crate) fn is_single(self) -> bool {
        self.mode == AdvanceMode::Single
    }
}

/// The normal condition that ended an advance operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdvanceStop {
    /// The request's fuel budget was exhausted.
    FuelLimit,
    /// The request's simulated-cycle budget was exhausted.
    CycleLimit,
    /// Execution reached a breakpoint at the given primary CPU program counter.
    Breakpoint(u32),
    /// The CPU reported zero forward progress.
    NoProgress,
}

/// Structured progress and stop accounting for one advance operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdvanceReport {
    /// The normal condition that ended the operation.
    pub stop: AdvanceStop,
    /// Fuel units consumed by primary scheduling quanta and idle skips.
    pub fuel_consumed: u64,
    /// Number of primary CPU scheduling quanta completed.
    pub primary_steps: u64,
    /// Number of secondary CPU steps completed.
    pub secondary_steps: u64,
    /// Simulated machine cycles elapsed, including peripheral tick costs.
    pub elapsed_cycles: u64,
    /// Simulated cycles advanced specifically through idle fast-forwarding.
    pub idle_cycles: u64,
    /// Successful CPU execution batches committed.
    pub cpu_batches: u64,
}

impl AdvanceReport {
    /// Builds a complete report without an intermediate partial state.
    pub(crate) fn new(
        stop: AdvanceStop,
        fuel_consumed: u64,
        primary_steps: u64,
        secondary_steps: u64,
        elapsed_cycles: u64,
        idle_cycles: u64,
        cpu_batches: u64,
    ) -> Self {
        Self {
            stop,
            fuel_consumed,
            primary_steps,
            secondary_steps,
            elapsed_cycles,
            idle_cycles,
            cpu_batches,
        }
    }
}
