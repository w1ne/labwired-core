// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The ONE verdict of a run.
//!
//! A run produces a single judgment. `result.json`'s `status`, the JUnit
//! summary, the stderr banner and the process exit code are four *views* of
//! that one judgment, not four judgments.
//!
//! They used to be computed separately. `execute_test_loop` had a `status`
//! chain and, 128 lines later, an exit-code chain built from a different set of
//! predicates, and the multi-node `run_world` had a second pair of its own. The
//! two single-machine chains had drifted apart in both directions:
//!
//! * `firmware_declared_failure` was only in the `status` chain, so firmware
//!   that ended its own run with `EXIT 5` produced `status: "fail"`, a `FAIL`
//!   banner — and exit code `0`, which
//!   `docs/simulation_protocol.md` §5 tells a CI runner to "Treat as CI
//!   Success".
//! * `fault_gate_failed` was only in the exit-code chain — it could not be in
//!   the other, because it was not computed until thirty lines after `status`
//!   had already been decided — so a `require_fault_fired` run whose fault never
//!   fired shipped `status: "pass"`, a `PASS` banner and a JUnit file with zero
//!   failures, while the process exited `1`.
//!
//! Either way round, a harness reading the artifact and a harness reading `$?`
//! reported opposite outcomes for the same silicon. For a tool sold as a
//! hardware oracle that is the worst available failure: not a wrong answer, but
//! two answers.
//!
//! So the decision is made exactly once, here, from [`RunFacts`]. The two views
//! are [`Verdict::status`] and [`Verdict::exit_code`], both derived from that
//! one value — divergence is no longer something you can express. Adding a new
//! way for a run to fail means adding a field to [`RunFacts`], which every
//! consumer of every view then inherits at once.

use std::process::ExitCode;

use crate::{EXIT_ASSERT_FAIL, EXIT_CONFIG_ERROR, EXIT_PASS, EXIT_RUNTIME_ERROR};

/// The judgment. One value per run.
///
/// The variants are the protocol's exit codes, named for their cause;
/// `docs/simulation_protocol.md` §5 is the source of both the codes and the
/// `status` spellings, and [`Verdict::status`] / [`Verdict::exit_code`] are the
/// only implementations of that table in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every assertion held and nothing invalidated the run.
    Pass,
    /// The firmware, or the script's assertions, said no.
    AssertionFail,
    /// The run never happened as configured, so it proved nothing.
    ConfigError,
    /// The simulation itself failed in a way the script did not expect.
    RuntimeError,
}

impl Verdict {
    /// The `status` field of `result.json`, the snapshot and the JUnit report.
    pub fn status(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::AssertionFail => "fail",
            Verdict::ConfigError | Verdict::RuntimeError => "error",
        }
    }

    /// The human-facing one-line banner label on stderr.
    pub fn banner_label(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::AssertionFail => "FAIL",
            Verdict::ConfigError | Verdict::RuntimeError => "ERROR",
        }
    }

    /// The process exit code. `docs/simulation_protocol.md` §5.
    pub fn exit_code(self) -> ExitCode {
        ExitCode::from(self.exit_status())
    }

    /// The exit code as a number, so tests can compare it. `ExitCode` is
    /// deliberately opaque and cannot be inspected.
    pub fn exit_status(self) -> u8 {
        match self {
            Verdict::Pass => EXIT_PASS,
            Verdict::AssertionFail => EXIT_ASSERT_FAIL,
            Verdict::ConfigError => EXIT_CONFIG_ERROR,
            Verdict::RuntimeError => EXIT_RUNTIME_ERROR,
        }
    }
}

/// Everything about a finished run that bears on its verdict.
///
/// Every field is a *reason the run is not a pass*; `Default` is therefore the
/// clean run. A runner fills in the ones its execution model can observe and
/// leaves the rest false — the multi-node runner has no `simctl` verdict and no
/// fault gate, and says so by omission rather than by keeping its own chain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunFacts {
    /// A declared stimulus was refused by the engine, so nothing the run
    /// observed can be attributed to it. Dominates every other reason: a
    /// "fail" from a run whose inputs never arrived is not a trustworthy fail
    /// either.
    pub stimuli_rejected: bool,
    /// The firmware ended its own run with a non-zero code through `simctl`.
    pub firmware_declared_failure: bool,
    /// At least one script assertion did not hold.
    pub assertions_failed: bool,
    /// `require_fault_fired` was set and some injected fault never took
    /// effect, so the run did not test what it claimed to test.
    pub fault_gate_failed: bool,
    /// The run hit a safety limit (wall time, UART cap, no progress) that no
    /// `expected_stop_reason` assertion accounted for.
    pub unexpected_safety_stop: bool,
    /// The simulation raised an error that no `expected_stop_reason` assertion
    /// accounted for.
    pub unrescued_runtime_error: bool,
}

impl RunFacts {
    /// The single decision. Order is precedence, highest first.
    pub fn verdict(&self) -> Verdict {
        if self.stimuli_rejected {
            Verdict::ConfigError
        } else if self.firmware_declared_failure
            || self.assertions_failed
            || self.fault_gate_failed
            || self.unexpected_safety_stop
        {
            Verdict::AssertionFail
        } else if self.unrescued_runtime_error {
            Verdict::RuntimeError
        } else {
            Verdict::Pass
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The protocol table, transcribed from `docs/simulation_protocol.md` §5
    /// and §4.1 rather than from the code above.
    ///
    /// | Exit | Constant             | status    |
    /// |------|----------------------|-----------|
    /// | 0    | `EXIT_PASS`          | `"pass"`  |
    /// | 1    | `EXIT_ASSERT_FAIL`   | `"fail"`  |
    /// | 2    | `EXIT_CONFIG_ERROR`  | `"error"` |
    /// | 3    | `EXIT_RUNTIME_ERROR` | `"error"` |
    #[test]
    fn the_two_views_agree_for_every_reachable_run() {
        // Exhaustive over the fact space: 2^6 = 64 runs, every combination of
        // reasons a run might not be a pass. No sampling, no representative
        // cases — if any combination could produce a status and an exit code
        // that tell different stories, it is in here.
        for bits in 0u8..64 {
            let facts = RunFacts {
                stimuli_rejected: bits & 1 != 0,
                firmware_declared_failure: bits & 2 != 0,
                assertions_failed: bits & 4 != 0,
                fault_gate_failed: bits & 8 != 0,
                unexpected_safety_stop: bits & 16 != 0,
                unrescued_runtime_error: bits & 32 != 0,
            };
            let verdict = facts.verdict();
            let expected_status = match verdict.exit_status() {
                0 => "pass",
                1 => "fail",
                2 | 3 => "error",
                other => panic!("{facts:?} produced exit code {other}, outside the protocol"),
            };
            assert_eq!(
                verdict.status(),
                expected_status,
                "{facts:?}: status {:?} and exit code {} disagree",
                verdict.status(),
                verdict.exit_status()
            );
            assert_eq!(verdict.banner_label(), verdict.status().to_uppercase());
        }
    }

    #[test]
    fn a_clean_run_passes() {
        assert_eq!(RunFacts::default().verdict(), Verdict::Pass);
        assert_eq!(RunFacts::default().verdict().exit_status(), 0);
    }

    /// The first half of the drift this module removed: the firmware's own
    /// verdict must reach the exit code, not only the artifact.
    #[test]
    fn a_firmware_declared_failure_is_a_failing_exit_code() {
        let facts = RunFacts {
            firmware_declared_failure: true,
            ..RunFacts::default()
        };
        assert_eq!(facts.verdict(), Verdict::AssertionFail);
        assert_eq!(facts.verdict().exit_status(), EXIT_ASSERT_FAIL);
        assert_eq!(facts.verdict().status(), "fail");
    }

    /// The second half: the fault gate must reach the artifact, not only the
    /// exit code.
    #[test]
    fn a_fault_gate_trip_is_a_failing_status() {
        let facts = RunFacts {
            fault_gate_failed: true,
            ..RunFacts::default()
        };
        assert_eq!(facts.verdict(), Verdict::AssertionFail);
        assert_eq!(facts.verdict().status(), "fail");
        assert_eq!(facts.verdict().exit_status(), EXIT_ASSERT_FAIL);
    }

    /// A rejected stimulus outranks everything, including a firmware pass.
    #[test]
    fn a_rejected_stimulus_dominates() {
        let facts = RunFacts {
            stimuli_rejected: true,
            assertions_failed: true,
            unrescued_runtime_error: true,
            ..RunFacts::default()
        };
        assert_eq!(facts.verdict(), Verdict::ConfigError);
        assert_eq!(facts.verdict().status(), "error");
        assert_eq!(facts.verdict().exit_status(), EXIT_CONFIG_ERROR);
    }

    /// An assertion failure outranks a runtime error: the firmware got a
    /// verdict, and a fault that also occurred does not rescue it.
    #[test]
    fn an_assertion_failure_outranks_a_runtime_error() {
        let facts = RunFacts {
            assertions_failed: true,
            unrescued_runtime_error: true,
            ..RunFacts::default()
        };
        assert_eq!(facts.verdict(), Verdict::AssertionFail);
    }
}
