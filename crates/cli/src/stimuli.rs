// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Command-line stimulus surface: parse `--stimulus` JSON arguments into the
//! shared `StimulusSpec` type and drive them through `Machine::set_input` at
//! `at_start` / `after_cycles`. Used by the system-aware `run --system` driver.

use labwired_config::{FaultTrigger, StimulusAction, StimulusSpec, StimulusTarget};
use labwired_core::{Cpu, Machine};
use serde::Deserialize;
use tracing::{error, info};

/// One `--stimulus` argument. Field names are the agent-facing MCP schema
/// (`channel`, `value`, `after_cycles`, `component`); unknown fields are
/// rejected so typos fail loudly instead of silently resetting to a default.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliStimulus {
    pub channel: String,
    pub value: f64,
    #[serde(default)]
    pub after_cycles: Option<u64>,
    #[serde(default)]
    pub component: Option<String>,
}

/// Parse one `--stimulus` argument into the shared `StimulusSpec` type.
/// `index` is the argument's position in `--stimulus` order and is included in
/// every error so a caller can fix the right entry.
pub fn parse_stimulus_arg(index: usize, raw: &str) -> Result<StimulusSpec, String> {
    let parsed: CliStimulus =
        serde_json::from_str(raw).map_err(|e| format!("--stimulus[{index}]: invalid JSON: {e}"))?;
    if parsed.channel.trim().is_empty() {
        return Err(format!("--stimulus[{index}]: channel cannot be empty"));
    }
    let trigger = match parsed.after_cycles {
        Some(cycles) if cycles > 0 => FaultTrigger::AfterCycles { cycles },
        _ => FaultTrigger::AtStart,
    };
    Ok(StimulusSpec {
        action: StimulusAction::Input {
            target: StimulusTarget {
                component: parsed.component,
                channel: parsed.channel,
            },
            value: parsed.value,
        },
        trigger,
    })
}

/// The name a stimulus addresses, for diagnostics: a SimInput channel or a
/// co-simulation signal path.
fn spec_channel(s: &StimulusSpec) -> &str {
    match &s.action {
        StimulusAction::Input { target, .. } => &target.channel,
        StimulusAction::CosimSignal(signal) => &signal.path,
    }
}

/// Apply one stimulus through the generic `Machine::set_input` path. The log
/// strings are asserted by `e2e_kw41z_cow_stimulus.rs`; do not reword them.
pub fn apply_spec<C: Cpu>(machine: &mut Machine<C>, s: &StimulusSpec) {
    match &s.action {
        StimulusAction::Input { target, value } => {
            let result = match target.component.as_deref() {
                Some(component) => machine.set_input_on(component, &target.channel, *value),
                None => machine.set_input(&target.channel, *value),
            };
            match result {
                Ok(()) => info!("stimulus: {} = {} applied", target.channel, value),
                Err(e) => error!(
                    "stimulus '{}' = {} could not be applied: {e}",
                    target.channel, value
                ),
            }
        }
        // The CLI has no co-simulation session; a cosim stimulus can only come
        // from a test script, never from `parse_stimulus_arg`.
        StimulusAction::CosimSignal(signal) => error!(
            "stimulus '{}' = {} could not be applied: run --system has no co-simulation session",
            signal.path, signal.value
        ),
    }
}

/// At-start application plus once-only `after_cycles` firing for the
/// system-aware `run --system` driver.
pub struct StimulusTrack {
    specs: Vec<StimulusSpec>,
    at_start_applied: bool,
    fired: Vec<bool>,
}

impl StimulusTrack {
    pub fn new(specs: &[StimulusSpec]) -> Self {
        Self {
            specs: specs.to_vec(),
            at_start_applied: false,
            fired: vec![false; specs.len()],
        }
    }

    /// Apply every `at_start` spec. Idempotent.
    pub fn apply_at_start<C: Cpu>(&mut self, machine: &mut Machine<C>) {
        if self.at_start_applied {
            return;
        }
        for s in &self.specs {
            if matches!(s.trigger, FaultTrigger::AtStart) {
                apply_spec(machine, s);
            }
        }
        self.at_start_applied = true;
    }

    /// Earliest unfired `after_cycles` threshold strictly after
    /// `current_cycle`. Deliberately filters past deadlines: a due-but-unfired
    /// spec (the caller's poll cycle source can lag `machine.total_cycles`
    /// during idle fast-forward) must not hide a later deadline from the
    /// batch clamp.
    pub fn next_deadline_after(&self, current_cycle: u64) -> Option<u64> {
        self.specs
            .iter()
            .zip(&self.fired)
            .filter_map(|(s, fired)| {
                if *fired {
                    return None;
                }
                match s.trigger {
                    FaultTrigger::AfterCycles { cycles } if cycles > current_cycle => Some(cycles),
                    _ => None,
                }
            })
            .min()
    }

    /// Unfired `after_cycles` specs that never reached their threshold:
    /// `(index, channel, deadline)` for end-of-run diagnostics. `at_start`
    /// specs are applied by `apply_at_start`, so they are never pending.
    pub fn pending(&self) -> Vec<(usize, &str, u64)> {
        self.specs
            .iter()
            .zip(&self.fired)
            .enumerate()
            .filter_map(|(i, (s, fired))| {
                if *fired {
                    return None;
                }
                match s.trigger {
                    FaultTrigger::AfterCycles { cycles } => Some((i, spec_channel(s), cycles)),
                    _ => None,
                }
            })
            .collect()
    }

    /// Apply every not-yet-fired `after_cycles` spec whose threshold `cycles`
    /// has reached.
    pub fn poll<C: Cpu>(&mut self, machine: &mut Machine<C>, cycles: u64) {
        if !self.has_pending() {
            return;
        }
        for i in self.due(cycles) {
            apply_spec(machine, &self.specs[i]);
        }
    }

    fn has_pending(&self) -> bool {
        self.specs
            .iter()
            .zip(&self.fired)
            .any(|(s, fired)| !*fired && matches!(s.trigger, FaultTrigger::AfterCycles { .. }))
    }

    /// Mark and return the indices newly due at `cycles` (test seam).
    fn due(&mut self, cycles: u64) -> Vec<usize> {
        let mut due = Vec::new();
        for (i, (s, fired)) in self.specs.iter().zip(self.fired.iter_mut()).enumerate() {
            if *fired {
                continue;
            }
            if let FaultTrigger::AfterCycles { cycles: threshold } = s.trigger {
                if cycles >= threshold {
                    *fired = true;
                    due.push(i);
                }
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::FaultTrigger;

    #[test]
    fn parses_the_agent_wire_shape() {
        let s = parse_stimulus_arg(
            0,
            r#"{"component":"fxos8700","channel":"x","value":2.0,"after_cycles":3000000}"#,
        )
        .expect("parse stimulus");
        let target = s.input_target().expect("input stimulus");
        assert_eq!(target.component.as_deref(), Some("fxos8700"));
        assert_eq!(target.channel, "x");
        assert_eq!(s.value(), 2.0);
        assert_eq!(s.trigger, FaultTrigger::AfterCycles { cycles: 3_000_000 });
    }

    #[test]
    fn omitted_or_zero_after_cycles_is_at_start() {
        let a = parse_stimulus_arg(0, r#"{"channel":"x","value":1.0}"#).expect("parse");
        assert_eq!(a.trigger, FaultTrigger::AtStart);
        let b = parse_stimulus_arg(0, r#"{"channel":"x","value":1.0,"after_cycles":0}"#)
            .expect("parse");
        assert_eq!(b.trigger, FaultTrigger::AtStart);
    }

    #[test]
    fn bad_json_reports_the_argument_index() {
        let err = parse_stimulus_arg(2, r#"{"channel":"x","value":1.0,"afterCycle":5}"#)
            .expect_err("unknown field must fail");
        assert!(err.starts_with("--stimulus[2]:"), "{err}");

        let err = parse_stimulus_arg(1, "not json").expect_err("malformed must fail");
        assert!(err.starts_with("--stimulus[1]:"), "{err}");

        let err = parse_stimulus_arg(0, r#"{"channel":"x","value":1.0,"after_cycles":-5}"#)
            .expect_err("negative cycles must fail");
        assert!(err.starts_with("--stimulus[0]:"), "{err}");
    }

    #[test]
    fn empty_channel_is_rejected() {
        let err = parse_stimulus_arg(0, r#"{"channel":"","value":1.0}"#).expect_err("empty");
        assert!(err.contains("channel"), "{err}");
    }

    fn spec(channel: &str, trigger: FaultTrigger) -> StimulusSpec {
        StimulusSpec {
            action: StimulusAction::Input {
                target: StimulusTarget {
                    component: None,
                    channel: channel.to_string(),
                },
                value: 1.0,
            },
            trigger,
        }
    }

    #[test]
    fn track_fires_each_spec_once_and_reports_deadlines() {
        let specs = vec![
            spec("x", FaultTrigger::AfterCycles { cycles: 100 }),
            spec("y", FaultTrigger::AfterCycles { cycles: 50 }),
            spec("z", FaultTrigger::AtStart),
        ];
        let mut track = StimulusTrack::new(&specs);

        assert_eq!(track.next_deadline_after(0), Some(50));
        assert_eq!(track.due(49), Vec::<usize>::new());
        assert_eq!(track.due(50), vec![1]);
        assert_eq!(track.next_deadline_after(50), Some(100));
        assert_eq!(track.due(200), vec![0]);
        assert_eq!(
            track.next_deadline_after(200),
            None,
            "fired specs never re-arm"
        );
        assert_eq!(track.due(500), Vec::<usize>::new());
    }

    #[test]
    fn pending_reports_unfired_after_cycles_specs() {
        let specs = vec![
            spec("x", FaultTrigger::AfterCycles { cycles: 100 }),
            spec("y", FaultTrigger::AfterCycles { cycles: 50 }),
            spec("z", FaultTrigger::AtStart),
        ];
        let mut track = StimulusTrack::new(&specs);
        assert_eq!(track.pending(), vec![(0, "x", 100), (1, "y", 50)]);
        track.due(50);
        assert_eq!(track.pending(), vec![(0, "x", 100)]);
        track.due(100);
        assert!(track.pending().is_empty(), "at_start is never pending");
    }

    #[test]
    fn next_deadline_after_skips_past_deadlines() {
        // A due-but-unfired spec (lagging poll source) must not hide a later
        // deadline from the batch clamp.
        let specs = vec![
            spec("a", FaultTrigger::AfterCycles { cycles: 100 }),
            spec("b", FaultTrigger::AfterCycles { cycles: 300 }),
        ];
        let track = StimulusTrack::new(&specs);
        assert_eq!(track.next_deadline_after(200), Some(300));
        assert_eq!(
            track.next_deadline_after(300),
            None,
            "strictly-after filter"
        );
    }
}
