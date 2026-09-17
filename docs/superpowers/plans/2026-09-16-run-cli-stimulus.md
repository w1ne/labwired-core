# `run --system --stimulus` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `labwired run` drive simulated input devices from the command line via repeatable `--stimulus` JSON arguments, without writing YAML.

**Architecture:** A new optional `--system` on `run` selects a system-aware driver (ARM only in this phase) that attaches external devices from the manifest and runs the firmware through the generic `Machine` loop. Stimuli are parsed into the existing `labwired_config::StimulusSpec` type and applied by a new `StimulusTrack` helper shared with `execute_test_loop`, so `run` and `test` cannot drift on trigger semantics. Pre-run validation uses a new read-only `SystemBus::resolve_input` and fails fast with the available channel inventory.

**Tech Stack:** Rust (clap, serde_json, serde), `labwired-core` / `labwired-config` / `labwired-loader` workspace crates, integration tests spawning the `labwired` binary.

**Spec:** `docs/superpowers/specs/2026-09-16-run-cli-stimulus-design.md`

**Conventions:**
- Run every command from `core/`.
- Exit codes: `EXIT_PASS=0`, `EXIT_ASSERT_FAIL=1`, `EXIT_CONFIG_ERROR=2`, `EXIT_RUNTIME_ERROR=3`.
- Commit after each task. Do not use `git add -A`; add the listed files only.
- The repo is currently on a detached HEAD; that is fine for committing.

## File Structure

| Action | Path | Responsibility |
| --- | --- | --- |
| Modify | `crates/core/src/bus/sim_inputs.rs` | Add read-only `resolve_input`; make `set_input` share it |
| Modify | `crates/core/tests/sim_input.rs` | Resolution + alias + ambiguity tests |
| Create | `crates/cli/src/stimuli.rs` | `--stimulus` parsing, `StimulusTrack`, exact apply logging |
| Modify | `crates/cli/src/main.rs` | `mod stimuli;`, `RunArgs` fields, dispatch, `execute_test_loop` refactor |
| Modify | `crates/cli/src/commands/mod.rs` | Register `run_system` |
| Create | `crates/cli/src/commands/run_system.rs` | System-aware driver: guards, validation, batched run loop |
| Modify | `crates/cli/src/commands/run.rs` | Branch on `--system`, `chip_path()` accessor updates |
| Create | `crates/cli/tests/e2e_run_stimulus.rs` | End-to-end guard-rail, validation, and KW41Z happy-path tests |

---

### Task 1: Core `SystemBus::resolve_input`

**Files:**
- Modify: `crates/core/src/bus/sim_inputs.rs` (around `list_inputs`/`set_input`, currently lines 182-262)
- Test: `crates/core/tests/sim_input.rs` (append at end)

- [ ] **Step 1: Write the failing tests**

Append to `crates/core/tests/sim_input.rs`:

```rust
#[test]
fn resolve_input_returns_metadata_without_applying() {
    let mut bus = kw41z_lcd_bus();
    // Latch x first: an unlatched FXOS8700 animation advances on every burst
    // read, so a before/after comparison would move for reasons unrelated to
    // resolution. A latched pose is stable and makes the check meaningful.
    bus.set_input(None, "x", 1.0).expect("latch x");
    let before = read_axis(&mut bus, 0x01);

    let ch = bus.resolve_input(None, "x").expect("resolve x");
    assert_eq!(ch.key, "x");
    assert_eq!(ch.unit, "g");
    assert_eq!((ch.min, ch.max), (-8.0, 8.0));

    // Resolution is read-only: the device's pose must be untouched.
    assert_eq!(read_axis(&mut bus, 0x01), before);

    match bus.resolve_input(None, "nope") {
        Err(SimInputError::NoDevice(c)) => assert_eq!(c, "nope"),
        other => panic!("expected NoDevice, got {other:?}"),
    }
}

#[test]
fn resolve_input_accepts_both_component_aliases() {
    let mut bus = f103_input_matrix_bus();

    // "distance" lives on both the VL53L1X (i2c1) and the HC-SR04 (sonar).
    match bus.resolve_input(None, "distance") {
        Err(SimInputError::Ambiguous { matches, .. }) => assert_eq!(matches, 2),
        other => panic!("expected Ambiguous, got {other:?}"),
    }

    // The peripheral bus name and the external-device id must both resolve.
    let by_bus = bus
        .resolve_input(Some("i2c1"), "distance")
        .expect("resolve by bus name");
    assert_eq!(by_bus.key, "distance");
    assert!(!by_bus.unit.is_empty());
    let by_id = bus
        .resolve_input(Some("tof"), "distance")
        .expect("resolve by device id");
    assert_eq!(by_id.key, "distance");

    // A component that doesn't own the channel is a NoDevice, not a fallback.
    match bus.resolve_input(Some("uart1"), "temperature") {
        Err(SimInputError::NoDevice(m)) => assert_eq!(m, "uart1/temperature"),
        other => panic!("expected NoDevice, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p labwired-core --test sim_input resolve_input`

Expected: compile error `error[E0599]: no method named 'resolve_input' found for struct 'SystemBus'`.

- [ ] **Step 3: Implement `resolve_input` and share it with `set_input`**

In `crates/core/src/bus/sim_inputs.rs`, insert this method immediately before `pub fn set_input`:

```rust
    /// Read-only resolution: the same match/uniqueness rules `set_input`
    /// enforces, returning the matching channel's metadata WITHOUT applying a
    /// value. The pre-run validation primitive for the CLI stimulus surface;
    /// `set_input` resolves through it so validation and dispatch cannot
    /// drift, and callers can range-check against `InputChannel::{min,max}`
    /// before a run starts.
    pub fn resolve_input(
        &mut self,
        component: Option<&str>,
        channel: &str,
    ) -> Result<crate::sim_input::InputChannel, crate::sim_input::SimInputError> {
        use crate::sim_input::SimInputError;
        let mut matches = 0usize;
        let mut found: Option<crate::sim_input::InputChannel> = None;
        self.for_each_sim_input(&mut |name, si| {
            if Self::component_matches(component, name, si) {
                if let Some(ch) = si.input_channels().iter().find(|c| c.key == channel) {
                    matches += 1;
                    found = Some(*ch);
                }
            }
            false
        });
        if matches == 0 {
            let missing = match component {
                Some(c) => format!("{c}/{channel}"),
                None => channel.to_string(),
            };
            return Err(SimInputError::NoDevice(missing));
        }
        if matches > 1 {
            return Err(SimInputError::Ambiguous {
                channel: channel.to_string(),
                matches,
            });
        }
        Ok(found.expect("exactly one match implies a found channel"))
    }
```

Then replace the head of `set_input` — delete its count/ambiguity block and delegate:

```rust
    pub fn set_input(
        &mut self,
        component: Option<&str>,
        channel: &str,
        value: f64,
    ) -> Result<(), crate::sim_input::SimInputError> {
        // Typed NoDevice / Ambiguous / UnknownChannel errors from the shared
        // resolution path; the apply walk below cannot re-hit them.
        self.resolve_input(component, channel)?;
        let mut result = Ok(());
        self.for_each_sim_input(&mut |name, si| {
            if Self::component_matches(component, name, si)
                && si.input_channels().iter().any(|c| c.key == channel)
            {
                result = si.set_input(channel, value);
                true
            } else {
                false
            }
        });
```

Keep the existing `sync_analog_inputs()` tail of `set_input` unchanged.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p labwired-core --test sim_input`

Expected: all tests PASS including the two new `resolve_input_*` tests and the existing `set_input_*` / `component_disambiguates_*` tests.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/bus/sim_inputs.rs crates/core/tests/sim_input.rs
git commit -m "feat(core): add read-only SystemBus::resolve_input for stimulus validation"
```

---

### Task 2: `crates/cli/src/stimuli.rs` — parsing and track

**Files:**
- Create: `crates/cli/src/stimuli.rs`
- Modify: `crates/cli/src/main.rs` (module declarations at the top, around line 7)

- [ ] **Step 1: Write the failing tests**

Create `crates/cli/src/stimuli.rs` with the license header, the test module, and no implementation yet (the file must compile as a test target):

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Command-line stimulus surface: parse `--stimulus` JSON arguments into the
//! shared `StimulusSpec` type and drive them through `Machine::set_input` at
//! `at_start` / `after_cycles`. Also used by `execute_test_loop`.

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
        assert_eq!(s.target.component.as_deref(), Some("fxos8700"));
        assert_eq!(s.target.channel, "x");
        assert_eq!(s.value, 2.0);
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
            target: StimulusTarget {
                component: None,
                channel: channel.to_string(),
            },
            trigger,
            value: 1.0,
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
        assert_eq!(track.next_deadline_after(200), None, "fired specs never re-arm");
        assert_eq!(track.due(500), Vec::<usize>::new());
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
        assert_eq!(track.next_deadline_after(300), None, "strictly-after filter");
    }
}
```

Add `mod stimuli;` to `crates/cli/src/main.rs` next to the existing `mod artifacts;` / `mod commands;` declarations.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p labwired-cli --bin labwired stimuli`

Expected: compile errors `cannot find function 'parse_stimulus_arg'` and `cannot find type 'StimulusSpec'`.

- [ ] **Step 3: Write the implementation**

Add to `crates/cli/src/stimuli.rs` above the test module:

```rust
use labwired_config::{FaultTrigger, StimulusSpec, StimulusTarget};
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
    let parsed: CliStimulus = serde_json::from_str(raw)
        .map_err(|e| format!("--stimulus[{index}]: invalid JSON: {e}"))?;
    if parsed.channel.trim().is_empty() {
        return Err(format!("--stimulus[{index}]: channel cannot be empty"));
    }
    let trigger = match parsed.after_cycles {
        Some(cycles) if cycles > 0 => FaultTrigger::AfterCycles { cycles },
        _ => FaultTrigger::AtStart,
    };
    Ok(StimulusSpec {
        target: StimulusTarget {
            component: parsed.component,
            channel: parsed.channel,
        },
        trigger,
        value: parsed.value,
    })
}

/// Apply one stimulus through the generic `Machine::set_input` path. The log
/// strings are asserted by `e2e_kw41z_cow_stimulus.rs`; do not reword them.
pub fn apply_spec<C: Cpu>(machine: &mut Machine<C>, s: &StimulusSpec) {
    let result = match s.target.component.as_deref() {
        Some(component) => machine.set_input_on(component, &s.target.channel, s.value),
        None => machine.set_input(&s.target.channel, s.value),
    };
    match result {
        Ok(()) => info!("stimulus: {} = {} applied", s.target.channel, s.value),
        Err(e) => error!(
            "stimulus '{}' = {} could not be applied: {:?}",
            s.target.channel, s.value, e
        ),
    }
}

/// At-start application plus once-only `after_cycles` firing, shared by the
/// test runner and the `run --system` driver.
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p labwired-cli --bin labwired stimuli`

Expected: 6 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/stimuli.rs crates/cli/src/main.rs
git commit -m "feat(cli): add stimulus parsing and StimulusTrack"
```

---

### Task 3: Refactor `execute_test_loop` onto `StimulusTrack`

**Files:**
- Modify: `crates/cli/src/main.rs` (`execute_test_loop`, three stimulus regions)

- [ ] **Step 1: Record the baseline**

Run: `cargo test -p labwired-cli --test e2e_kw41z_cow_stimulus --test e2e_leo_airquality`

Expected: PASS. Record the test counts; both files must pass unchanged after the refactor.

- [ ] **Step 2: Replace the setup block**

In `execute_test_loop`, replace the block beginning with the comment `// Declarative input stimuli (schema_version 1.2)...` and ending with the `pending_stimuli` collection (currently around lines 1482-1510) with:

```rust
    // Declarative input stimuli (schema_version 1.2), driven through the
    // generic `Machine::set_input` path; the same `StimulusTrack` powers
    // `run --system`.
    let mut stimulus_track = crate::stimuli::StimulusTrack::new(stimuli);
    stimulus_track.apply_at_start(machine);
```

- [ ] **Step 3: Replace the per-iteration fire block**

Replace the block beginning `// Fire any 'after_cycles' stimulus whose threshold the run has reached.` (currently around lines 1601-1615) with:

```rust
        // Fire any `after_cycles` stimulus whose threshold the run has reached.
        stimulus_track.poll(machine, metrics.get_cycles());
```

- [ ] **Step 4: Replace the batch deadline clamp**

Replace the `for (stimulus, fired) in &pending_stimuli { ... }` loop (currently around lines 1656-1666) with:

```rust
        let current_cycle = machine.total_cycles;
        if let Some(deadline) = stimulus_track.next_deadline_after(current_cycle) {
            limit = limit.min(deadline - current_cycle);
        }
```

- [ ] **Step 5: Run the baseline tests to verify no behavior changed**

Run: `cargo test -p labwired-cli --test e2e_kw41z_cow_stimulus --test e2e_leo_airquality`

Expected: PASS, same test counts. In particular `e2e_kw41z_cow_stimulus::stimulus_flips_the_cow_active_mid_run` still asserts the stderr line `stimulus: x = 2`.

- [ ] **Step 6: Run the whole CLI suite**

Run: `cargo test -p labwired-cli`

Expected: PASS (the refactor removed the only direct uses of `pending_stimuli`; if the compiler flags the old `apply_stimulus` closure as unused, ensure it was deleted with the replaced blocks).

- [ ] **Step 7: Commit**

```bash
git add crates/cli/src/main.rs
git commit -m "refactor(cli): drive test-loop stimuli through shared StimulusTrack"
```

---

### Task 4: `run --system` surface, driver, and end-to-end tests

**Files:**
- Modify: `crates/cli/src/main.rs` (`RunArgs`, dispatch)
- Modify: `crates/cli/src/commands/mod.rs`
- Modify: `crates/cli/src/commands/run.rs`
- Create: `crates/cli/src/commands/run_system.rs`
- Create: `crates/cli/tests/e2e_run_stimulus.rs`

- [ ] **Step 1: Write the failing end-to-end tests**

Create `crates/cli/tests/e2e_run_stimulus.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for `labwired run --system --stimulus`: guard rails,
// fail-fast validation with a machine-readable channel inventory, and the
// KW41Z cow-activity happy path (the command-line twin of the
// examples/kw41z-cow-activity/stimulus-shake.yaml test-script run).

use std::path::PathBuf;
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("execute labwired")
}

const KW41Z_SYSTEM: &str = "configs/systems/frdm-kw41z-lcd.yaml";
const KW41Z_FIRMWARE: &str = "tests/fixtures/kw41z-lcd-activity.elf";
const SHAKE: &str =
    r#"{"component":"fxos8700","channel":"x","value":2.0,"after_cycles":3000000}"#;

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn stimulus_requires_a_system_manifest() {
    let out = run_cli(&[
        "run",
        "--chip",
        "configs/chips/stm32f103.yaml",
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--stimulus",
        SHAKE,
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("--system"), "{}", stderr_of(&out));
}

#[test]
fn system_run_requires_an_explicit_step_budget() {
    let out = run_cli(&["run", "--system", KW41Z_SYSTEM, "--firmware", KW41Z_FIRMWARE]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("--max-steps"), "{}", stderr_of(&out));
}

#[test]
fn gpio_trace_is_rejected_on_the_system_driver() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--gpio-trace",
        "/tmp/labwired-gpio-should-not-exist.jsonl",
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("gpio-trace"), "{}", stderr_of(&out));
}

#[test]
fn unsupported_arch_is_rejected() {
    let out = run_cli(&[
        "run",
        "--system",
        "configs/systems/esp32-wroom-32.yaml",
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("Xtensa"), "{}", stderr_of(&out));
}

#[test]
fn unknown_channel_fails_fast_with_the_available_inventory() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--json",
        "--stimulus",
        r#"{"channel":"nope","value":1.0}"#,
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_of(&out)).expect("--json error payload on stdout");
    assert_eq!(parsed["exit_code"], 2);
    let available = parsed["details"]["available"]
        .as_array()
        .expect("available inventory");
    assert!(
        available
            .iter()
            .any(|e| e["component"] == "fxos8700" && e["channel"] == "x"),
        "{}",
        stdout_of(&out)
    );
}

#[test]
fn out_of_range_value_fails_fast() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "10",
        "--stimulus",
        r#"{"component":"fxos8700","channel":"x","value":99.0}"#,
    ]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(stderr_of(&out).contains("value"), "{}", stderr_of(&out));
}

#[test]
fn kw41z_cow_reacts_to_a_command_line_stimulus() {
    let out = run_cli(&[
        "run",
        "--system",
        KW41Z_SYSTEM,
        "--firmware",
        KW41Z_FIRMWARE,
        "--max-steps",
        "6000000",
        "--stimulus",
        SHAKE,
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("stimulus: x = 2"),
        "expected the applied-stimulus log\n{}",
        stderr_of(&out)
    );
    let uart = stdout_of(&out);
    assert!(uart.contains("MOOD=CALM"), "{uart}");
    assert!(uart.contains("MOOD=ACTIVE"), "{uart}");
    let first_calm = uart.find("MOOD=CALM").unwrap();
    let first_active = uart.find("MOOD=ACTIVE").unwrap();
    assert!(first_calm < first_active, "CALM must precede ACTIVE");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p labwired-cli --test e2e_run_stimulus`

Expected: failures — `error: unexpected argument '--system' found` (clap exit 2) for the system tests, and `stimulus_requires_a_system_manifest` failing because `run` ignores `--stimulus` today.

- [ ] **Step 3: Extend `RunArgs` and dispatch**

In `crates/cli/src/main.rs`, change the `RunArgs` struct field `chip` and add two fields:

```rust
    /// Path to the chip descriptor YAML. Required unless --system is given
    /// (the manifest then names the chip).
    #[arg(long, required_unless_present = "system", conflicts_with = "system")]
    pub chip: Option<PathBuf>,

    /// Board manifest (SystemManifest YAML). Selects the system-aware driver:
    /// external devices from the manifest are attached, and the chip comes
    /// from the manifest rather than --chip.
    #[arg(long, value_name = "PATH")]
    pub system: Option<PathBuf>,

    /// Declarative input stimulus as JSON (repeatable), agent MCP shape:
    /// {"channel":"x","value":2.0,"after_cycles":3000000,"component":"fxos8700"}.
    /// Requires --system; after_cycles omitted or 0 applies at start.
    #[arg(long = "stimulus", value_name = "JSON")]
    pub stimulus: Vec<String>,
```

Add this accessor below the `RunArgs` definition:

```rust
impl RunArgs {
    /// The chip descriptor path. Clap guarantees `--chip` when `--system` is
    /// absent; the system driver resolves the chip from the manifest instead.
    pub(crate) fn chip_path(&self) -> &Path {
        self.chip
            .as_deref()
            .expect("clap enforces --chip unless --system is given")
    }
}
```

Update the dispatch arm in `main()`:

```rust
        Some(Commands::Run(args)) => commands::run::run_firmware(args, cli.json),
```

- [ ] **Step 4: Update `run.rs` for the optional chip and the system branch**

In `crates/cli/src/commands/run.rs`:

1. Change the signature and add the branch at the top of `run_firmware`:

```rust
pub(crate) fn run_firmware(args: RunArgs, json: bool) -> ExitCode {
    use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
    use labwired_core::bus::SystemBus;
    use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};
    use labwired_core::SimulationError;

    if args.system.is_some() {
        return super::run_system::run_firmware_with_system(&args, json);
    }

    if !args.stimulus.is_empty() {
        crate::emit_error(
            json,
            "ConfigError",
            "--stimulus requires --system: stimuli target input devices declared in a system manifest"
                .to_string(),
            None,
            crate::EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(crate::EXIT_CONFIG_ERROR);
    }

    let chip_path = args.chip_path().to_path_buf();

    // Read the chip YAML to validate the chip family.
    let chip_yaml = match std::fs::read_to_string(&chip_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read chip YAML at {:?}: {e}", chip_path);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
```

2. Replace every remaining `args.chip` use with the accessor so it compiles with `Option`:
   - `run_firmware_riscv` (lines ~55, ~68): `&args.chip` → `args.chip_path()`; `args.chip.to_string_lossy()` → `args.chip_path().to_string_lossy()`.
   - `run_firmware` Xtensa branch (line ~537): `args.chip` in the error format → `args.chip_path()`.
   - `run_firmware_arm` (lines ~1119, ~1130): `args.chip.display()` → `args.chip_path().display()`, `args.chip.to_string_lossy()` → `args.chip_path().to_string_lossy()`.
   - `run_firmware_arm` also uses `args.chip` in the synthesized manifest comment/build — same replacement.
   - `run_firmware_esp32` does not read the chip path; leave it.

3. Register the module in `crates/cli/src/commands/mod.rs`:

```rust
pub mod run_system;
```

- [ ] **Step 5: Implement the driver**

Create `crates/cli/src/commands/run_system.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! System-aware `labwired run` driver: attach the external devices declared in
//! a SystemManifest, drive the firmware with the generic `Machine` loop, and
//! apply `--stimulus` entries through the shared `StimulusTrack`.
//!
//! Phase 1 supports ARM (Cortex-M) manifests only. RISC-V and Xtensa boot
//! preparation lives in the test runner and is an explicit follow-up; an
//! unsupported arch fails fast instead of running a partially-booted system.

use crate::stimuli::{parse_stimulus_arg, StimulusTrack};
use crate::{emit_error, RunArgs, EXIT_CONFIG_ERROR, EXIT_PASS, EXIT_RUNTIME_ERROR};

use labwired_config::{Arch, ChipDescriptor, StimulusSpec, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::num::NonZeroU32;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

/// Instructions per batched `advance`; keeps stimulus deadlines and UART
/// output responsive without single-stepping multi-million-cycle runs.
const RUN_BATCH_CAP: u64 = 10_000;

pub(crate) fn run_firmware_with_system(args: &RunArgs, json: bool) -> ExitCode {
    let Some(system_path) = args.system.as_deref() else {
        unreachable!("run_firmware dispatches here only when --system is set");
    };
    let Some(max_steps) = args.max_steps else {
        emit_error(
            json,
            "ConfigError",
            "run --system requires an explicit --max-steps budget".to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    };
    if args.gpio_trace.is_some() {
        emit_error(
            json,
            "ConfigError",
            "--gpio-trace is only supported on the ESP32-S3 run path; it cannot be combined with --system"
                .to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let specs = match parse_all_stimuli(&args.stimulus) {
        Ok(specs) => specs,
        Err((index, message)) => {
            emit_error(
                json,
                "ConfigError",
                message,
                Some(serde_json::json!({ "stimulus_index": index })),
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let manifest = match SystemManifest::from_file(system_path) {
        Ok(m) => m,
        Err(e) => {
            emit_error(
                json,
                "ConfigError",
                format!("cannot parse system manifest {}: {e:#}", system_path.display()),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    let chip_path = system_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&manifest.chip);
    let chip = match ChipDescriptor::from_file(&chip_path) {
        Ok(c) => c,
        Err(e) => {
            emit_error(
                json,
                "ConfigError",
                format!("cannot parse chip descriptor {}: {e:#}", chip_path.display()),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if chip.arch != Arch::Arm {
        emit_error(
            json,
            "ConfigError",
            format!(
                "run --system currently supports ARM manifests only; got arch {:?} from {} \
                 (RISC-V and Xtensa boot paths are a follow-up)",
                chip.arch,
                chip_path.display()
            ),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let mut bus = match SystemBus::from_config(&chip, &manifest) {
        Ok(b) => b,
        Err(e) => {
            emit_error(
                json,
                "ConfigError",
                format!("cannot build system bus: {e:#}"),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Echo UART to stdout, same as the existing `run` paths.
    let uart_sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(uart_sink, true);

    let program = match labwired_loader::load_elf(&args.firmware) {
        Ok(p) => p,
        Err(e) => {
            emit_error(
                json,
                "LoadError",
                format!("cannot load firmware ELF {:?}: {e}", args.firmware),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    if let Err(e) = machine.load_firmware(&program) {
        emit_error(
            json,
            "LoadError",
            format!("cannot map firmware into bus: {e}"),
            None,
            EXIT_RUNTIME_ERROR,
        );
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    if let Err(errors) = validate_stimuli(&specs, &mut machine.bus) {
        let available = inventory(&mut machine.bus);
        let message = errors
            .iter()
            .filter_map(|e| e["error"].as_str())
            .collect::<Vec<_>>()
            .join("; ");
        emit_error(
            json,
            "ConfigError",
            format!("stimulus validation failed: {message}"),
            Some(serde_json::json!({ "errors": errors, "available": available })),
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let mut track = StimulusTrack::new(&specs);
    track.apply_at_start(&mut machine);

    let mut steps: u64 = 0;
    while steps < max_steps {
        track.poll(&mut machine, machine.total_cycles);
        let current_cycle = machine.total_cycles;
        let mut limit = (max_steps - steps).min(RUN_BATCH_CAP);
        if let Some(deadline) = track.next_deadline_after(current_cycle) {
            limit = limit.min(deadline - current_cycle);
        }
        let batch = limit.max(1);
        let request = labwired_core::AdvanceRequest::run(Some(batch))
            .with_batch_cap(
                NonZeroU32::new(batch.min(u64::from(u32::MAX)) as u32)
                    .expect("batch is non-zero"),
            )
            .with_breakpoints(labwired_core::BreakpointPolicy::Ignore);
        match machine.advance(request) {
            Ok(report) => {
                steps += report.primary_steps;
                if report.primary_steps == 0 && report.idle_cycles == 0 {
                    break; // halt
                }
            }
            Err(e) => {
                crate::commands::run::export_bus_trace_if_requested(
                    &args.bus_trace_out,
                    &machine.bus,
                );
                eprintln!("labwired run (system): simulation error at step {steps}: {e}");
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
    }

    crate::commands::run::export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
    eprintln!(
        "labwired-cli run (system): reached --max-steps {max_steps}; pc=0x{:08x}",
        machine.cpu.get_pc()
    );
    ExitCode::from(EXIT_PASS)
}

/// Parse every `--stimulus` argument; the first failure carries its index.
fn parse_all_stimuli(raw: &[String]) -> Result<Vec<StimulusSpec>, (usize, String)> {
    let mut out = Vec::with_capacity(raw.len());
    for (i, arg) in raw.iter().enumerate() {
        match parse_stimulus_arg(i, arg) {
            Ok(spec) => out.push(spec),
            Err(message) => return Err((i, message)),
        }
    }
    Ok(out)
}

/// Resolve and range-check every spec against the live bus without applying
/// anything. Returns all failures so one message can list them.
fn validate_stimuli(
    specs: &[StimulusSpec],
    bus: &mut SystemBus,
) -> Result<(), Vec<serde_json::Value>> {
    let mut errors = Vec::new();
    for (i, s) in specs.iter().enumerate() {
        match bus.resolve_input(s.target.component.as_deref(), &s.target.channel) {
            Ok(ch) => {
                if !s.value.is_finite() || s.value < ch.min || s.value > ch.max {
                    errors.push(serde_json::json!({
                        "stimulus_index": i,
                        "channel": s.target.channel,
                        "error": format!(
                            "stimulus[{i}]: value {} outside [{}, {}] {}",
                            s.value, ch.min, ch.max, ch.unit
                        ),
                    }));
                }
            }
            Err(e) => errors.push(serde_json::json!({
                "stimulus_index": i,
                "channel": s.target.channel,
                "error": format!("stimulus[{i}]: {e:?}"),
            })),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The `{component, channel}` drive points the firmware's board exposes, for
/// agent self-correction on validation failures.
fn inventory(bus: &mut SystemBus) -> Vec<serde_json::Value> {
    bus.list_inputs()
        .into_iter()
        .map(|(component, ch)| {
            serde_json::json!({
                "component": component,
                "channel": ch.key,
                "unit": ch.unit,
                "min": ch.min,
                "max": ch.max,
            })
        })
        .collect()
}
```

Note: `RunArgs` is defined in `main.rs` and `run_system` is a submodule of the binary crate; if `use crate::{emit_error, RunArgs, ...}` fails because `RunArgs` is not re-exported, import it as `use crate::RunArgs;` (it is defined at crate root, so this works) and `use crate::commands::run::export_bus_trace_if_requested;` instead of the fully qualified path.

- [ ] **Step 6: Run the end-to-end tests to verify they pass**

Run: `cargo test -p labwired-cli --test e2e_run_stimulus -- --nocapture`

Expected: 7 tests PASS. The KW41Z test prints `MOOD=CALM` then `MOOD=ACTIVE` on stdout and `stimulus: x = 2 applied` on stderr.

- [ ] **Step 7: Run the CLI and core suites for regressions**

Run: `cargo test -p labwired-cli && cargo test -p labwired-core --test sim_input`

Expected: PASS. The no-`--system` paths are unchanged except that `--chip` is now optional in help text.

- [ ] **Step 8: Commit**

```bash
git add crates/cli/src/main.rs crates/cli/src/commands/mod.rs crates/cli/src/commands/run.rs crates/cli/src/commands/run_system.rs crates/cli/tests/e2e_run_stimulus.rs
git commit -m "feat(cli): add run --system --stimulus command-line stimuli"
```

---

### Task 5: Help text, formatting, and final verification sweep

**Files:**
- Modify: `crates/cli/src/main.rs` (help strings only, if Step 1 finds them lacking)

- [ ] **Step 1: Verify the help surface**

Run: `cargo run -q -p labwired-cli -- run --help`

Expected output contains `--system <PATH>`, `--stimulus <JSON>`, and the chip help stating it is required unless `--system` is given. If any is missing, fix the doc comments on the `RunArgs` fields and re-run.

- [ ] **Step 2: Verify the guard messages by hand**

Run: `cargo run -q -p labwired-cli -- run --system configs/systems/frdm-kw41z-lcd.yaml --firmware tests/fixtures/kw41z-lcd-activity.elf`

Expected: exit 2, stderr mentions `--max-steps` (no hang, no partial run).

- [ ] **Step 3: Format, lint, and run the full validation sweep**

Run from `core/`:

```bash
cargo fmt --all -- --check
cargo clippy -p labwired-cli -p labwired-core -- -D warnings
cargo test -p labwired-core --test sim_input
cargo test -p labwired-cli
```

Expected: all PASS, no clippy warnings. If `cargo fmt` reports diffs, run `cargo fmt --all` and re-commit.

- [ ] **Step 4: Commit any help-text/formatting fixes**

```bash
git add crates/cli/src/main.rs
git commit -m "docs(cli): clarify run --system and --stimulus help text"
```

---

## Self-Review

**Spec coverage:**

| Spec requirement | Task |
| --- | --- |
| `--system` selects system-aware driver; chip from manifest | Task 4 (steps 3-5) |
| `--chip` required-unless/conflicts rules | Task 4 step 3 |
| `--stimulus` MCP JSON shape, repeatable, omitted/0 = at_start | Task 2 |
| Fail-fast validation + `--json` inventory | Task 1 (`resolve_input`), Task 4 step 5 |
| Shared `StimulusTrack`, behavior-preserving test-loop refactor | Tasks 2-3 |
| Unbounded run refused (`--max-steps` required) | Task 4 steps 1, 5 |
| `--gpio-trace` rejected with `--system` | Task 4 steps 1, 5 |
| ARM-only arch gate | Task 4 steps 1, 5 |
| No behavior change without `--system` | Task 4 steps 3-4, regression in step 7 |
| Exact log strings preserved | Task 2 (`apply_spec`), Task 3 step 5 |
| Tests: unit, e2e, arch/guard rails, alias resolution | Tasks 1, 2, 4 |
| Validation plan commands | Task 4 step 7, Task 5 step 3 |

**Placeholder scan:** no TBD/TODO; every code step shows complete code; every command has expected output.

**Type consistency:** `resolve_input` returns `InputChannel` (Task 1) and is consumed with `.key/.unit/.min/.max` in Task 4; `parse_stimulus_arg(index, raw)` and `StimulusTrack::{new, apply_at_start, next_deadline_after, poll}` (Task 2) are used with those exact names in Task 4; `run_firmware(args, json)` and `RunArgs::chip_path()` (Task 4 step 3) match every call site updated in step 4; `run_firmware_with_system(&args, json)` matches the dispatch call.
