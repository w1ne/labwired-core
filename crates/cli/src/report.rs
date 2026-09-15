// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! What a finished run REPORTS — separate from what drove it.
//!
//! Remediation row 6.11 splits the CLI into verdict / report / drive.
//! `verdict.rs` already existed; this is the report half, lifted out of a
//! 4,719-line `lib.rs`.
//!
//! Both functions here answer "what do we tell the caller", and neither should
//! need to know how the run was configured. `report_metrics` took `&Cli` — the
//! whole clap surface — to read one boolean, so it now takes the boolean.
//!
//! [`build_stop_reason_details`] is the one that mattered. It decides which
//! LIMIT ended a run and which observation crossed it, it has four call sites
//! across `lib.rs` and `commands/environment_test.rs`, and it had no tests. A
//! run that stops is reported to a human as "max_steps: 20000" or
//! "wall_time_ms: 5000"; naming the wrong pair there sends someone tuning a
//! budget that was never the binding one. It is pure — inputs to a struct, no
//! I/O — so the only reason it was untested is that it lived in the middle of
//! a file nothing could reach into.

use tracing::info;

use crate::artifacts::{NamedU64, StopReasonDetails};
use labwired_config::{StopReason, TestLimits};

pub(crate) fn report_metrics<C: labwired_core::Cpu>(
    json: bool,
    cpu: &C,
    metrics: &labwired_core::metrics::PerformanceMetrics,
) {
    if json {
        let report = serde_json::json!({
            "status": "finished",
            "final_pc": cpu.get_pc(),
            "total_instructions": metrics.get_instructions(),
            "total_cycles": metrics.get_cycles(),
            "average_ips": metrics.get_ips(),
        });
        println!("{}", serde_json::to_string(&report).unwrap());
    } else {
        info!("Simulation loop finished.");
        info!("Final PC: {:#x}", cpu.get_pc());
        info!("Total Instructions: {}", metrics.get_instructions());
        info!("Total Cycles: {}", metrics.get_cycles());
        info!("Average IPS: {:.2}", metrics.get_ips());
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_stop_reason_details(
    stop_reason: &StopReason,
    limits: &TestLimits,
    steps_executed: u64,
    cycles: u64,
    uart_bytes: u64,
    stuck_steps: u64,
    duration: std::time::Duration,
    vcd_bytes: u64,
) -> StopReasonDetails {
    let (triggered_limit, observed) = match stop_reason {
        StopReason::MaxSteps => (
            Some(NamedU64 {
                name: "max_steps".to_string(),
                value: limits.max_steps,
            }),
            Some(NamedU64 {
                name: "steps_executed".to_string(),
                value: steps_executed,
            }),
        ),
        StopReason::MaxCycles => (
            limits.max_cycles.map(|v| NamedU64 {
                name: "max_cycles".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "cycles".to_string(),
                value: cycles,
            }),
        ),
        StopReason::MaxUartBytes => (
            limits.max_uart_bytes.map(|v| NamedU64 {
                name: "max_uart_bytes".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "uart_bytes".to_string(),
                value: uart_bytes,
            }),
        ),
        StopReason::NoProgress => (
            limits.no_progress_steps.map(|v| NamedU64 {
                name: "no_progress_steps".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "stuck_steps".to_string(),
                value: stuck_steps,
            }),
        ),
        StopReason::WallTime => (
            limits.wall_time_ms.map(|v| NamedU64 {
                name: "wall_time_ms".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "elapsed_wall_time_ms".to_string(),
                value: duration.as_millis().min(u128::from(u64::MAX)) as u64,
            }),
        ),
        StopReason::MaxVcdBytes => (
            limits.max_vcd_bytes.map(|v| NamedU64 {
                name: "max_vcd_bytes".to_string(),
                value: v,
            }),
            Some(NamedU64 {
                name: "vcd_bytes".to_string(),
                value: vcd_bytes,
            }),
        ),
        StopReason::AssertionsPassed => (None, None),
        // No limit triggered this one — the firmware chose to end the run. The
        // exit code is reported separately as `firmware_exit_code`, not as a
        // limit/observation pair.
        StopReason::FirmwareExit => (None, None),
        StopReason::MemoryViolation
        | StopReason::DecodeError
        | StopReason::Halt
        | StopReason::Exception
        | StopReason::ConfigError => (None, None),
    };

    StopReasonDetails {
        triggered_stop_condition: stop_reason.clone(),
        triggered_limit,
        observed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn limits() -> TestLimits {
        TestLimits {
            max_steps: 20_000,
            max_cycles: Some(1_000_000),
            max_uart_bytes: Some(4_096),
            no_progress_steps: Some(500),
            wall_time_ms: Some(5_000),
            max_vcd_bytes: Some(1_048_576),
            stop_when_assertions_pass: false,
            stop_when_assertions_pass_settle_steps: 0,
            stop_when_assertions_pass_min_steps: 0,
        }
    }

    /// steps=1, cycles=2, uart=3, stuck=4, wall=5ms, vcd=6 — every observation
    /// distinct, so an arm that reads the wrong one cannot pass by coincidence.
    fn details(stop: StopReason) -> StopReasonDetails {
        build_stop_reason_details(&stop, &limits(), 1, 2, 3, 4, Duration::from_millis(5), 6)
    }

    /// Each limit-triggered stop must name ITS limit and ITS observation. This
    /// is the whole job: a run that ends is reported to a human as
    /// "max_steps: 20000", and naming the wrong pair sends someone tuning a
    /// budget that was never the binding one.
    #[test]
    fn every_limit_stop_names_its_own_limit_and_observation() {
        let cases = [
            (
                StopReason::MaxSteps,
                "max_steps",
                20_000,
                "steps_executed",
                1,
            ),
            (StopReason::MaxCycles, "max_cycles", 1_000_000, "cycles", 2),
            (
                StopReason::MaxUartBytes,
                "max_uart_bytes",
                4_096,
                "uart_bytes",
                3,
            ),
            (
                StopReason::NoProgress,
                "no_progress_steps",
                500,
                "stuck_steps",
                4,
            ),
            (
                StopReason::WallTime,
                "wall_time_ms",
                5_000,
                "elapsed_wall_time_ms",
                5,
            ),
            (
                StopReason::MaxVcdBytes,
                "max_vcd_bytes",
                1_048_576,
                "vcd_bytes",
                6,
            ),
        ];
        for (stop, limit_name, limit_value, obs_name, obs_value) in cases {
            let d = details(stop.clone());
            let limit = d
                .triggered_limit
                .unwrap_or_else(|| panic!("{stop:?} must name a limit"));
            let obs = d
                .observed
                .unwrap_or_else(|| panic!("{stop:?} must name an observation"));
            assert_eq!(limit.name, limit_name, "{stop:?} named the wrong limit");
            assert_eq!(
                limit.value, limit_value,
                "{stop:?} reported the wrong limit value"
            );
            assert_eq!(obs.name, obs_name, "{stop:?} named the wrong observation");
            assert_eq!(
                obs.value, obs_value,
                "{stop:?} reported the wrong observation"
            );
        }
    }

    /// A limit that was never configured cannot be reported as the one that
    /// triggered. `max_cycles: None` with a MaxCycles stop is a contradiction,
    /// and the honest answer is "no limit named" rather than a fabricated zero.
    #[test]
    fn an_unset_limit_is_reported_as_absent_not_as_zero() {
        let mut l = limits();
        l.max_cycles = None;
        let d = build_stop_reason_details(
            &StopReason::MaxCycles,
            &l,
            1,
            2,
            3,
            4,
            Duration::from_millis(5),
            6,
        );
        assert!(
            d.triggered_limit.is_none(),
            "an unset limit must not be invented"
        );
        // The observation still stands: the run really did execute 2 cycles.
        assert_eq!(d.observed.expect("observation").value, 2);
    }

    /// Stops that no limit caused must name neither. FirmwareExit especially:
    /// the firmware chose to end, and its exit code is reported separately, so
    /// attaching a limit here would claim the run was cut short when it was not.
    #[test]
    fn stops_no_limit_caused_name_neither_a_limit_nor_an_observation() {
        for stop in [
            StopReason::AssertionsPassed,
            StopReason::FirmwareExit,
            StopReason::MemoryViolation,
            StopReason::DecodeError,
            StopReason::Halt,
            StopReason::Exception,
            StopReason::ConfigError,
        ] {
            let d = details(stop.clone());
            assert!(d.triggered_limit.is_none(), "{stop:?} must name no limit");
            assert!(d.observed.is_none(), "{stop:?} must name no observation");
        }
    }

    /// The reported stop reason is the one asked about — cheap, and it is the
    /// field every consumer keys off.
    #[test]
    fn the_reported_stop_reason_is_the_one_passed_in() {
        assert_eq!(
            details(StopReason::WallTime).triggered_stop_condition,
            StopReason::WallTime
        );
    }

    /// Wall time is the only observation that is derived rather than passed
    /// through, so it gets its own check that the conversion is not lossy in
    /// the range a run can reach.
    #[test]
    fn wall_time_is_reported_in_milliseconds() {
        let d = build_stop_reason_details(
            &StopReason::WallTime,
            &limits(),
            0,
            0,
            0,
            0,
            Duration::from_secs(3),
            0,
        );
        assert_eq!(d.observed.expect("observation").value, 3_000);
    }
}
