// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! THE CONTRACT: a `#![cfg(feature = "event-scheduler")]` integration test must
//! run in a lane that gates a merge, or say in writing why it does not.
//!
//! # The hole this closes
//!
//! `crates/core/tests/` holds integration files whose whole body is behind
//! `#![cfg(feature = "event-scheduler")]`. With the feature off — the default
//! for every `cargo test` in this repo — the file compiles to nothing and
//! `cargo test --test <name>` prints `0 passed; 0 filtered out` and exits 0. A
//! lane that does not pass the feature therefore does not merely skip the file:
//! it reports it GREEN.
//!
//! **That last sentence is now false, deliberately.** Every file in this set
//! carries a `[[test]] required-features` block, so cargo SKIPS the target
//! instead of faking a pass, and a bare `--test <name>` is a hard error. That
//! contract is enforced across the whole workspace by
//! `no_vacuous_test_targets.rs`. It closes the "reports GREEN" half. This file
//! still owns the other half — **which lane actually runs it** — and that half
//! has no mechanical fix.
//!
//! The only lane that ran all of them was `core-full`, which is
//! schedule/dispatch/`[full-ci]` only — not push, not PR. So a fidelity
//! regression could land on `main` with every required check green and stay
//! there until someone happened to read a nightly log.
//!
//! It did. core#823 was a two-file YAML change that added `board_io` to
//! `configs/systems/nrf52840-dk.yaml`. The button's released level, driven at
//! attach, presented itself to the per-tick GPIO edge detector as a rising
//! edge; the per-edge scheduler harvest then re-armed a wake on EVERY
//! scheduler-driven peripheral, including the RADIO, which cannot latch
//! anything from a GPIO edge but did have a wake in flight. The duplicate fired
//! inside the same drain as the EasyDMA event and ran the air-time countdown to
//! completion on the spot: `EVENTS_END` moved from cycle 33 to cycle 2 and
//! stopped scaling with packet length at all. The two tests that assert BLE
//! 1 Mbit air time live in `nrf52_timer_walk_differential.rs` — nightly-only.
//!
//! # Why this gate is STATIC
//!
//! The question is not "does the test pass" — CI answers that. It is "does any
//! merge-gating lane RUN it", which is a property of `core-ci.yml`, not of the
//! simulator. Reading both files answers it in milliseconds, on every PR, with
//! no runner cost and nothing to be vacuously green about.
//!
//! # How a name counts as covered
//!
//! `core-ci.yml` has exactly one job that does not gate a merge: `core-full`
//! (schedule / `workflow_dispatch` / `[full-ci]`). The scan reads the file only
//! UP TO that job, so a `--test <name>` it finds is necessarily inside a lane
//! that runs on `pull_request` or on push to `main` (`pr-gate`,
//! `pr-scheduler-observable`, `core-integrity`, `ci-runner-image`). `full:` is
//! the last job in the file, and a companion test fails if that stops being
//! true — otherwise a job appended after it would be silently excluded and its
//! names would stop counting.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Integration tests that are deliberately NOT in a merge-gating lane. Each
/// entry needs a reason that survives review — "slow" with a measured number,
/// or "the lane would be vacuous". Adding a name here is a decision, which is
/// the entire point: the previous arrangement made it a silence.
const NIGHTLY_ONLY: &[(&str, &str)] = &[
    (
        "stm32f401_walk_differential",
        "9.6s of runtime (measured, debug). The four STM32 walk differentials \
         together are ~55s — most of the uncovered set's cost in four files. \
         They run in core-full; the cheap half runs pre-merge.",
    ),
    (
        "stm32h563_walk_differential",
        "12.9s of runtime (measured, debug); see stm32f401_walk_differential.",
    ),
    (
        "stm32l073_walk_differential",
        "14.3s of runtime (measured, debug); see stm32f401_walk_differential.",
    ),
    (
        "stm32l476_walk_differential",
        "18.6s of runtime (measured, debug); see stm32f401_walk_differential.",
    ),
    (
        "nrf54l15_idle_ff_speedup",
        "43.6s of runtime (measured, debug): it measures an idle fast-forward \
         speedup, so the wall clock IS the assertion and cannot be shortened.",
    ),
    (
        "bench_walk_free_kw41z",
        "Every test in the file is #[ignore]d (it is a benchmark). A default \
         lane would run zero of them and report green — a vacuous gate is worse \
         than none. It belongs to the --ignored lane.",
    ),
    (
        "esp32c3_clamped_full_state_differential",
        "Every test in the file is #[ignore]d; see bench_walk_free_kw41z.",
    ),
    (
        "esp32c3_oled_profile",
        "Every test in the file is #[ignore]d; see bench_walk_free_kw41z.",
    ),
    (
        "esp32c3_shipped_lab_batch_gate",
        "Every test in the file is #[ignore]d; see bench_walk_free_kw41z.",
    ),
    (
        "riscv_jit_c3_oled_differential",
        "Every test in the file is #[ignore]d, AND it needs `jit` as well as \
         `event-scheduler`; see bench_walk_free_kw41z.",
    ),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Integration test files whose ENTIRE body is behind the `event-scheduler`
/// feature (inner attribute), i.e. the ones a feature-less lane reports green
/// without running.
fn scheduler_gated_tests() -> BTreeSet<String> {
    let dir = repo_root().join("crates/core/tests");
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("read crates/core/tests") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read test file");
        // Inner attribute only: `#![cfg(...)]` gates the whole file. A
        // per-item `#[cfg(...)]` inside an otherwise-running file is not this
        // bug class — the file still executes.
        let gated = src.lines().any(|l| {
            let l = l.trim();
            l.starts_with("#![cfg(")
                && l.contains("feature = \"event-scheduler\"")
                && !l.contains("not(")
        });
        if gated {
            out.insert(
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .expect("test file stem")
                    .to_string(),
            );
        }
    }
    assert!(
        !out.is_empty(),
        "found no #![cfg(feature = \"event-scheduler\")] integration tests — the \
         scan is looking in the wrong place, not the repo that lost them"
    );
    out
}

fn core_ci_yaml() -> String {
    std::fs::read_to_string(repo_root().join(".github/workflows/core-ci.yml"))
        .expect("read core-ci.yml")
}

/// Every `--test <name>` named in a MERGE-GATING job of `core-ci.yml`, i.e.
/// everything above the `full:` job (see the module docs).
fn tests_named_in_core_ci() -> BTreeSet<String> {
    let src = core_ci_yaml();
    let gating = src
        .split_once("\n  full:\n")
        .map(|(before, _)| before)
        .expect("core-ci.yml must still define the `full:` job");
    let mut out = BTreeSet::new();
    let mut rest = gating;
    while let Some(pos) = rest.find("--test ") {
        rest = &rest[pos + "--test ".len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.insert(name);
        }
    }
    out
}

/// The premise of `tests_named_in_core_ci` is that `full:` — the one job in
/// `core-ci.yml` that does not gate a merge — is LAST, so "everything above it"
/// is exactly the gating set. A job appended after `full:` would be cut out of
/// the scan and its `--test` names would stop counting, quietly shrinking the
/// coverage this file claims to prove.
#[test]
fn core_full_must_stay_the_last_job_in_core_ci() {
    let src = core_ci_yaml();
    let after = src
        .split_once("\n  full:\n")
        .map(|(_, after)| after)
        .expect("core-ci.yml must still define the `full:` job");
    let later_job = after
        .lines()
        .find(|l| {
            let bytes = l.as_bytes();
            bytes.len() > 3
                && l.starts_with("  ")
                && !l.starts_with("   ")
                && l.trim_end().ends_with(':')
                && l.trim()
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':')
        })
        .map(str::trim);
    assert_eq!(
        later_job, None,
        "a job now follows `full:` in core-ci.yml. `tests_named_in_core_ci` \
         reads only what precedes `full:`, so that job's `--test` names no \
         longer count as coverage. Move it above `full:`, or teach the scan to \
         read each job's `if:` condition."
    );
}

/// A scheduler-gated integration test must be named in `core-ci.yml` (which
/// means a PR or push-to-main lane runs it) or be listed in `NIGHTLY_ONLY` with
/// a reason.
#[test]
fn every_scheduler_gated_test_has_a_merge_gating_lane_or_a_written_reason() {
    let gated = scheduler_gated_tests();
    let named = tests_named_in_core_ci();
    let excused: BTreeSet<&str> = NIGHTLY_ONLY.iter().map(|(n, _)| *n).collect();

    let orphaned: Vec<&String> = gated
        .iter()
        .filter(|t| !named.contains(*t) && !excused.contains(t.as_str()))
        .collect();

    assert!(
        orphaned.is_empty(),
        "these #![cfg(feature = \"event-scheduler\")] integration tests run in NO \
         merge-gating lane:\n  {}\n\nWith the feature off they do not fail, they \
         report GREEN with zero tests run — so nothing on a PR or on push to main \
         is checking them. Either add `--test <name>` to a lane in \
         .github/workflows/core-ci.yml (`pr-scheduler-observable` for the cheap \
         ones, `core-integrity` for the push backstop), or add the name to \
         NIGHTLY_ONLY in this file WITH a reason. Do not delete this assertion: \
         core#823 collapsed nRF52840 BLE air time from 32 cycles to 2 and shipped \
         to main green, because nrf52_timer_walk_differential lived in exactly \
         this gap.",
        orphaned
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// A `NIGHTLY_ONLY` entry must name a real file that is really scheduler-gated,
/// and must not also be covered by a lane. Otherwise the table rots into a list
/// of excuses for tests that no longer exist or no longer need one.
#[test]
fn nightly_only_entries_are_real_and_still_needed() {
    let gated = scheduler_gated_tests();
    let named = tests_named_in_core_ci();
    for (name, reason) in NIGHTLY_ONLY {
        assert!(
            gated.contains(*name),
            "NIGHTLY_ONLY names `{name}`, which is not a \
             #![cfg(feature = \"event-scheduler\")] integration test in \
             crates/core/tests (renamed? deleted? no longer feature-gated?). \
             Remove the entry."
        );
        assert!(
            !named.contains(*name),
            "NIGHTLY_ONLY excuses `{name}`, but core-ci.yml already runs it in a \
             gating lane. Drop the excuse."
        );
        assert!(
            reason.len() > 40,
            "NIGHTLY_ONLY entry `{name}` needs a reason someone can argue with, \
             not a label"
        );
    }
}
