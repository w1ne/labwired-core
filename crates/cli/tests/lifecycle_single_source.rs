// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! THE CONTRACT: `Machine` is the only place the simulation lifecycle lives.
//! No `labwired` CLI run path may rebuild it out of `Cpu::step` +
//! `SystemBus::tick_peripherals_*`.
//!
//! # The bug class this exists to prevent
//!
//! A "run loop" written as
//!
//! ```text
//! while steps < limit {
//!     cpu.step(&mut bus, &observers, &config)?;
//!     bus.tick_peripherals_with_costs();
//!     steps += 1;
//! }
//! ```
//!
//! looks complete and is not. It reproduces the *legacy-walk* half of
//! [`labwired_core::Machine::advance`] and silently drops the other half:
//!
//! * it never publishes the bus cycle clock
//!   (`SystemBus::set_current_cycle`), and
//! * it has no event scheduler at all, because the scheduler heap is a
//!   `Machine` field that a bare bus cannot reach.
//!
//! Both omissions are invisible in a default build, because with the
//! `event-scheduler` feature OFF the per-cycle walk drives every peripheral
//! and neither mechanism does anything. With the feature ON the walk SKIPS
//! every `uses_scheduler()` peripheral, so those two mechanisms become the
//! *only* thing that advances them — and a hand-rolled loop starves the lot.
//!
//! That is not hypothetical. It is exactly how eleven ESP32-classic / ESP32-S3
//! Tier-1 cells went `pass -> blocked` under
//! `--features event-scheduler` while every ARM and RISC-V cell stayed green:
//! ARM and RISC-V `run` already went through `Machine`, and the two Xtensa
//! `run` paths did not. `esp32::timg` never left cycle 0 (so `TIER1 clock` /
//! `TIER1 timer` reported `timg0-not-advancing` / `timg0-not-counting`), and
//! the shared `EspUart` TX FIFO — whose drain under the feature is owned by
//! `on_event` — never emptied, so the ESP32-S3 boot ROM spun forever inside
//! `uart_tx_one_char_uart` and no cell reported at all.
//!
//! `Machine::step`'s own doc comment already states the rule ("Frontends ...
//! must not reproduce the lifecycle with direct `Cpu::step` calls"). A doc
//! comment is not a gate. This is the gate.
//!
//! # Why this is a STATIC scan and not a behavioural test
//!
//! Every behavioural test in this crate builds a `Machine` — that is how the
//! test harness gets a machine at all. So a behavioural test exercises the
//! code path that was already correct and passes identically before and after
//! the fix. The defect is the *existence of a second implementation*, which is
//! a property of the source, not of any single run. Reading the source is what
//! distinguishes them.
//!
//! # Scope
//!
//! `crates/cli/src` only: the shipped `labwired` binary's run paths. Core's own
//! tests legitimately drive `tick()` and `Cpu::step` directly to pin
//! peripheral behaviour, and `crates/core/src/tests/walk_starvation_contract.rs`
//! already governs the peripheral side of this contract.

use std::path::{Path, PathBuf};

/// Workspace root = two parents up from the cli crate (crates/cli → core).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read cli src dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Strip `//`-style comments so the prose that DOCUMENTS the banned shape
/// (including this file's own module docs and the explanatory comment in
/// `commands/run.rs`) is not itself a violation. Block comments are not used
/// for prose in this crate; string literals containing these tokens would be
/// flagged, which is acceptable — none exist and one would be worth a look.
fn strip_line_comments(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Every banned token, with the reason it is banned.
const BANNED: &[(&str, &str)] = &[
    (
        "tick_peripherals_",
        "drives the peripheral walk directly; `Machine::commit_advance_boundary` \
         owns the peripheral boundary, and it also publishes the cycle clock and \
         drains the event scheduler, which this call does not",
    ),
    (
        ".step(&mut",
        "calls `Cpu::step` directly (its first argument is the bus); use \
         `Machine::step` / `Machine::advance`, which run the whole lifecycle",
    ),
];

#[test]
fn cli_run_paths_do_not_fork_the_machine_lifecycle() {
    let src = workspace_root().join("crates/cli/src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);

    // A scan that found no files to scan is a green light for nothing.
    assert!(
        files.len() >= 10,
        "expected to scan the cli crate's sources, found only {} file(s) under {} \
         — the gate is not looking at anything",
        files.len(),
        src.display(),
    );

    let mut violations = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read cli source");
        for (lineno, line) in text.lines().enumerate() {
            let code = strip_line_comments(line);
            for (token, why) in BANNED {
                if code.contains(token) {
                    violations.push(format!(
                        "{}:{}: `{}` — {}\n      {}",
                        file.strip_prefix(workspace_root())
                            .unwrap_or(file)
                            .display(),
                        lineno + 1,
                        token,
                        why,
                        line.trim(),
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "a CLI run path is rebuilding the machine lifecycle by hand:\n\n  {}\n\n\
         Route it through `labwired_core::Machine` (`Machine::step` for a \
         one-instruction quantum, `Machine::advance`/`Machine::run` for a bounded \
         run) instead. A hand-rolled `cpu.step` + `tick_peripherals_*` loop omits \
         `SystemBus::set_current_cycle` and the event scheduler, which starves \
         every `uses_scheduler()` peripheral under `--features event-scheduler` \
         — silently, because a default build's per-cycle walk hides it.",
        violations.join("\n\n  "),
    );
}
