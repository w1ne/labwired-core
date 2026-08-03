// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A workspace member that no pre-merge lane compiles is a member where bugs
//! ship behind a green board.
//!
//! `cargo clippy --all-targets` and `cargo test --lib` in `pr-gate` resolve to
//! the workspace's `default-members`. Anything outside that set is compiled by
//! NOTHING before a merge — and `crates/wasm`, the browser layer, sat outside it.
//! The browser is the only consumer that reaches the engine through the wasm
//! bindings, so a fork that exists only there is invisible to every other gate.
//! That is not hypothetical: an inlined device-type alias table in
//! `crates/wasm/src/inspect.rs` dropped one of three spellings, and a
//! `gxepd2_290_c90c` panel attached, was driven, and rendered dark.
//!
//! The contract asserted here
//! ==========================
//! Every workspace member that is (a) not a `default-member` and (b) buildable
//! on the host must be named explicitly in `pr-gate`, or carry a reason in
//! `HOST_UNBUILDABLE_PREFIXES` saying why it cannot be.
//!
//! Firmware crates are the honest exception: they are cross-compiled to
//! `thumbv*`/`riscv*`/`xtensa` targets and cannot link on a CI host at all. The
//! `full` lane builds them with explicit `--target` lines. Listing them here
//! would force either a false claim or a permanently-red gate, so they are
//! exempted BY PREFIX with the reason recorded — not silently skipped.
//!
//! Adding a new non-firmware member therefore fails this test until someone
//! decides which it is. That decision is the point; the previous default was to
//! make it by accident.

use std::path::{Path, PathBuf};

/// Members exempt from the pre-merge lane, and why. Prefix-matched on the
/// workspace-relative path.
const HOST_UNBUILDABLE_PREFIXES: &[(&str, &str)] = &[
    (
        "crates/firmware",
        "cross-compiled guest firmware (thumbv*/riscv*/xtensa); cannot link on a CI \
         host. Built with explicit --target lines in the `full` lane.",
    ),
    (
        "crates/riscv-ci-fixture",
        "guest firmware for the RISC-V fixture; same story as crates/firmware-*.",
    ),
    (
        "examples/",
        "the example labs are guest firmware crates built for embedded targets, not \
         host libraries. `full` cross-builds the ones CI covers.",
    ),
];

/// Host-buildable crates that NO pre-merge lane compiles today.
///
/// This is a ratchet, not an approval. Each of these is compiled only by
/// `integrity`'s `cargo test --workspace --lib`, which runs on push to main —
/// so the first thing to notice a break is main itself, after the merge. That is
/// the same hole `crates/wasm` had; these are simply lower-blast-radius, because
/// nothing ships to a user through them alone.
///
/// The list may SHRINK and may never grow. Moving one into pr-gate is the fix;
/// adding a new entry means a new crate was created outside every pre-merge lane
/// and someone should say why in review rather than discovering it on main.
const PRE_MERGE_UNCOVERED: &[&str] = &[
    "crates/codegen",
    "crates/config",
    "crates/egress-relay",
    "crates/gdbstub",
    "crates/hw-oracle",
    "crates/hw-oracle-macros",
    "crates/hw-runner",
    "crates/hw-trace",
    "crates/ir",
    "crates/python",
    "crates/svd-ingestor",
    "crates/validation-report",
];

fn workspace_root() -> PathBuf {
    // crates/core -> crates -> <workspace root>
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Members and default-members, straight out of the root manifest.
///
/// Parsed with a small hand-rolled reader rather than a toml dependency: this
/// crate does not otherwise need one, and the two arrays are plain string lists.
fn workspace_member_lists() -> (Vec<String>, Vec<String>) {
    let manifest = std::fs::read_to_string(workspace_root().join("Cargo.toml"))
        .expect("read workspace Cargo.toml");

    let list_after = |key: &str| -> Vec<String> {
        let start = manifest
            .find(key)
            .unwrap_or_else(|| panic!("workspace Cargo.toml has no `{key}`"));
        let rest = &manifest[start..];
        let open = rest.find('[').expect("list opens");
        let close = rest.find(']').expect("list closes");
        rest[open + 1..close]
            .split(',')
            .filter_map(|s| {
                let s = s.trim().trim_matches('"').trim();
                (!s.is_empty() && !s.starts_with('#')).then(|| s.to_string())
            })
            .collect()
    };

    (list_after("members = "), list_after("default-members = "))
}

/// The package name declared by the crate at `path`.
///
/// The lane names packages (`-p labwired-wasm`); the manifest lists paths. This
/// is the join between them, and doing it by reading the file means a rename
/// cannot quietly break the match.
fn package_name(path: &Path) -> Option<String> {
    let toml = std::fs::read_to_string(path.join("Cargo.toml")).ok()?;
    for line in toml.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("name") {
            if let Some(eq) = rest.find('=') {
                return Some(rest[eq + 1..].trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// The body of the `pr-gate` job, so a `-p <crate>` in a nightly-only lane
/// cannot be mistaken for pre-merge coverage.
fn pr_gate_body() -> String {
    let ci = std::fs::read_to_string(workspace_root().join(".github/workflows/core-ci.yml"))
        .expect("read core-ci.yml");
    let start = ci
        .find("\n  pr-gate:")
        .expect("core-ci.yml defines a pr-gate job");
    let rest = &ci[start + 1..];
    // Job keys sit at exactly two spaces of indent; the next one ends this job.
    let end = rest
        .match_indices("\n  ")
        .find(|(i, _)| {
            let after = &rest[i + 3..];
            after
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && after
                    .lines()
                    .next()
                    .is_some_and(|l| l.trim_end().ends_with(':'))
        })
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn every_host_buildable_member_is_compiled_before_merge() {
    let (members, default_members) = workspace_member_lists();
    assert!(
        members.len() > 10,
        "parsed only {} workspace members — the manifest reader is broken, and a \
         broken reader here reports success while checking nothing",
        members.len()
    );
    assert!(
        default_members.contains(&"crates/core".to_string()),
        "parsed default-members does not contain crates/core: {default_members:?}"
    );

    let gate = pr_gate_body();
    assert!(
        gate.contains("cargo clippy"),
        "pr-gate body did not parse — found no clippy step in it"
    );

    let mut uncovered: Vec<String> = Vec::new();
    let mut unexpected: Vec<String> = Vec::new();
    for member in &members {
        if default_members.contains(member) {
            continue;
        }
        if HOST_UNBUILDABLE_PREFIXES
            .iter()
            .any(|(prefix, _)| member.starts_with(prefix))
        {
            continue;
        }
        let Some(pkg) = package_name(&workspace_root().join(member)) else {
            continue;
        };
        if !gate.contains(&format!("-p {pkg}")) {
            uncovered.push(format!("{member} ({pkg})"));
            if !PRE_MERGE_UNCOVERED.contains(&member.as_str()) {
                unexpected.push(format!("{member} ({pkg})"));
            }
        }
    }

    // Shrink-only. An entry that is now covered must leave the list, or the list
    // stops describing reality and starts excusing it.
    let stale: Vec<&str> = PRE_MERGE_UNCOVERED
        .iter()
        .copied()
        .filter(|m| !uncovered.iter().any(|u| u.starts_with(&format!("{m} "))))
        .collect();
    assert!(
        stale.is_empty(),
        "PRE_MERGE_UNCOVERED lists crates that ARE now compiled before merge: {stale:?}. \
         Delete them from the list — a ratchet that keeps satisfied entries stops \
         ratcheting."
    );

    assert!(
        unexpected.is_empty(),
        "these workspace members are outside `default-members` and are named by NO \
         step in pr-gate, so nothing compiles them before a merge:\n  {}\n\n\
         Either add a `-p <pkg>` step to pr-gate in .github/workflows/core-ci.yml, \
         or — if the crate genuinely cannot build on a CI host — add its path prefix \
         to HOST_UNBUILDABLE_PREFIXES with the reason. Do not leave it in neither \
         list: that is how crates/wasm ended up compiled by nothing and a dark-panel \
         bug shipped behind a green board.",
        unexpected.join("\n  ")
    );
}

#[test]
fn the_browser_layer_specifically_is_in_the_pre_merge_lane() {
    // The general contract above would be satisfied by moving crates/wasm into
    // default-members, into the exemption list, or out of the workspace. This
    // names it directly, because it is the member whose absence actually cost
    // something and the one most likely to be dropped again for build time.
    let gate = pr_gate_body();
    assert!(
        gate.contains("-p labwired-wasm"),
        "pr-gate no longer compiles crates/wasm. The browser layer is the only \
         consumer that reaches the engine through the wasm bindings, so a fork \
         that exists only there is invisible to every other pre-merge gate."
    );
}
