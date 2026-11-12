// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A workspace member that no pre-merge lane compiles is a member where bugs
//! ship behind a green board.
//!
//! Bare `cargo clippy --all-targets` and `cargo test --lib` resolve to
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
//! on the host must be named explicitly in a required job (or a worker whose
//! success that job enforces), or carry a reason in
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

/// Jobs that gate a MERGE, and therefore the only places a `-p <crate>` counts
/// as pre-merge coverage.
///
/// ⚠️ THIS LIST MUST MATCH main's REQUIRED STATUS CHECKS. A job that runs on
/// `pull_request` but is not required is advisory: it goes red and the merge
/// button stays green. Listing one here without adding it to branch protection
/// turns this whole file into a claim nobody enforces.
///
/// `browser-layer` was split out of `pr-gate` on 2026-08-21 because pr-gate had
/// outgrown its 12-minute timeout and was being killed inside its last step.
/// The split is only safe BECAUSE the new job was added to the required
/// contexts in the same change; see the note above `browser-layer` in
/// core-ci.yml.
const MERGE_BLOCKING_JOBS: &[&str] = &["pr-gate", "browser-layer", "pr-core-feature-off"];

const LIBRARY_AGGREGATE: &str = "pr-core-feature-off";
const LIBRARY_WORKERS: &[(&str, &str)] = &[
    ("pr-default-members-lib", "DEFAULT_MEMBERS_RESULT"),
    ("pr-core-feature-off-tests", "FEATURE_OFF_RESULT"),
];

/// Admit only the two existing library workers, and only after proving that
/// their required aggregate fails for every unsuccessful prerequisite result.
/// Naming a worker in `needs` alone is not proof: a skipped/advisory aggregate
/// or a guard that checks the wrong result would otherwise hide a coverage gap.
fn required_library_worker_bodies(ci: &str) -> Result<Vec<String>, String> {
    let workflow: serde_yaml::Value = serde_yaml::from_str(ci).map_err(|e| e.to_string())?;
    let jobs = &workflow["jobs"];
    let aggregate = &jobs[LIBRARY_AGGREGATE];
    if !MERGE_BLOCKING_JOBS.contains(&LIBRARY_AGGREGATE) {
        return Err("library aggregate is not a required job".into());
    }
    let needs: Vec<_> = aggregate["needs"]
        .as_sequence()
        .ok_or("library aggregate must declare both worker dependencies")?
        .iter()
        .map(|v| v.as_str())
        .collect();
    if needs
        != LIBRARY_WORKERS
            .iter()
            .map(|(job, _)| Some(*job))
            .collect::<Vec<_>>()
    {
        return Err("library aggregate dependencies differ from the proven workers".into());
    }
    let normalize = |text: &str| text.split_whitespace().collect::<String>();
    let condition = normalize(aggregate["if"].as_str().unwrap_or_default());
    if condition
        != "always()&&(github.event_name=='pull_request'||github.event_name=='workflow_dispatch')"
    {
        return Err("library aggregate must always check both results on pull requests".into());
    }
    let advisory = |node: &serde_yaml::Value| {
        !node["continue-on-error"].is_null() && node["continue-on-error"].as_bool() != Some(false)
    };
    if advisory(aggregate) {
        return Err("library aggregate cannot be advisory".into());
    }
    let steps = aggregate["steps"]
        .as_sequence()
        .ok_or("library aggregate must run its result guard")?;
    if steps.len() != 1 {
        return Err("library aggregate must contain the single proven result guard".into());
    }
    let guard = &steps[0];
    if advisory(guard) || !guard["if"].is_null() || !guard["uses"].is_null() {
        return Err("library result guard cannot be conditional, advisory or delegated".into());
    }
    if !guard["shell"].is_null() && guard["shell"].as_str() != Some("bash") {
        return Err("library result guard must use the tested bash shell".into());
    }
    let script = guard["run"]
        .as_str()
        .ok_or("library result guard has no script")?;
    let mut bodies = Vec::new();
    for (job, variable) in LIBRARY_WORKERS {
        if guard["env"][*variable].as_str()
            != Some(format!("${{{{ needs.{job}.result }}}}").as_str())
        {
            return Err(format!(
                "library result guard does not bind {job}'s actual result"
            ));
        }
        let worker = &jobs[*job];
        if advisory(worker)
            || normalize(worker["if"].as_str().unwrap_or_default())
                != "github.event_name=='pull_request'||github.event_name=='workflow_dispatch'"
        {
            return Err(format!(
                "{job} must run on every pull request without allowing failure"
            ));
        }
        // Only unconditional, non-advisory run steps count as coverage.
        // YAML comments and conditionally skipped commands add no coverage.
        let commands: Vec<_> = worker["steps"]
            .as_sequence()
            .ok_or_else(|| format!("{job} has no steps"))?
            .iter()
            .filter(|step| step["if"].is_null() && !advisory(step))
            .filter_map(|step| step["run"].as_str())
            .collect();
        bodies.push(commands.join("\n"));
    }
    // Exercise the actual workflow script, not a Rust reimplementation. Missing,
    // failed, cancelled and skipped workers must all keep the merge gate red.
    for first in ["success", "failure", "cancelled", "skipped", ""] {
        for second in ["success", "failure", "cancelled", "skipped", ""] {
            let result = std::process::Command::new("bash")
                .args([
                    "--noprofile",
                    "--norc",
                    "-e",
                    "-o",
                    "pipefail",
                    "-c",
                    script,
                ])
                .env_clear()
                .env(LIBRARY_WORKERS[0].1, first)
                .env(LIBRARY_WORKERS[1].1, second)
                .output()
                .map_err(|e| format!("execute actual library result guard: {e}"))?;
            if result.status.success() != (first == "success" && second == "success") {
                return Err(format!(
                    "library guard admits the wrong result for {first:?}/{second:?}"
                ));
            }
        }
    }
    Ok(bodies)
}

/// The body of one job in core-ci.yml.
fn job_body(job: &str) -> String {
    let ci = std::fs::read_to_string(workspace_root().join(".github/workflows/core-ci.yml"))
        .expect("read core-ci.yml");
    let start = ci
        .find(&format!("\n  {job}:"))
        .unwrap_or_else(|| panic!("core-ci.yml defines a {job} job"));
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

/// Every merge-blocking job's body, concatenated. A crate compiled by any of
/// them is compiled before a merge; one compiled only by a nightly or
/// push-to-main lane is not, and must not read as covered here.
fn pre_merge_body() -> String {
    let mut bodies: Vec<String> = MERGE_BLOCKING_JOBS.iter().map(|j| job_body(j)).collect();
    // Each job must actually run on pull_request. A job that lost that trigger
    // would still contribute its `-p` lines and silently keep this file green.
    for (job, body) in MERGE_BLOCKING_JOBS.iter().zip(&bodies) {
        assert!(
            body.contains("pull_request"),
            "{job} is in MERGE_BLOCKING_JOBS but does not run on pull_request, so \
             nothing it names is covered before a merge"
        );
    }
    let ci = std::fs::read_to_string(workspace_root().join(".github/workflows/core-ci.yml"))
        .expect("read core-ci.yml");
    bodies.extend(
        required_library_worker_bodies(&ci).expect("prove required library worker coverage"),
    );
    bodies.join("\n")
}

#[test]
fn library_worker_coverage_rejects_broken_dependency_proofs() {
    let ci = std::fs::read_to_string(workspace_root().join(".github/workflows/core-ci.yml"))
        .expect("read core-ci.yml");
    let original: serde_yaml::Value = serde_yaml::from_str(&ci).unwrap();
    let bodies =
        required_library_worker_bodies(&ci).expect("actual aggregate enforces both workers");
    assert!(bodies[0].contains("cargo test --lib"));
    assert!(bodies[1].contains("cargo test -p nrf-softdevice-hle --features capi"));

    for mutation in 0..7 {
        let mut changed = original.clone();
        let aggregate = &mut changed["jobs"][LIBRARY_AGGREGATE];
        match mutation {
            0 => {
                aggregate["needs"].as_sequence_mut().unwrap().pop().unwrap();
            }
            1 => {
                aggregate["steps"][0]["env"]["FEATURE_OFF_RESULT"] =
                    "${{ needs.pr-gate.result }}".into()
            }
            2 => aggregate["continue-on-error"] = true.into(),
            3 => aggregate["steps"][0]["run"] = "exit 0\n".into(),
            4 => aggregate["if"] = "github.event_name == 'pull_request'".into(),
            5 => changed["jobs"][LIBRARY_WORKERS[1].0]["continue-on-error"] = true.into(),
            6 => aggregate["steps"][0]["continue-on-error"] = true.into(),
            _ => unreachable!(),
        }
        assert!(
            required_library_worker_bodies(&serde_yaml::to_string(&changed).unwrap()).is_err(),
            "broken dependency proof mutation {mutation} was admitted"
        );
    }
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

    let gate = pre_merge_body();
    assert!(
        gate.contains("cargo clippy"),
        "merge-blocking job bodies did not parse — found no clippy step in them"
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
         required job or proven worker, so nothing compiles them before a merge:\n  {}\n\n\
         Either add a `-p <pkg>` step to a required job or proven worker in .github/workflows/core-ci.yml, \
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
    let gate = pre_merge_body();
    assert!(
        gate.contains("-p labwired-wasm"),
        "no merge-blocking job compiles crates/wasm. The browser layer is the only \
         consumer that reaches the engine through the wasm bindings, so a fork \
         that exists only there is invisible to every other pre-merge gate. It \
         lives in the `browser-layer` job; if you moved it again, move it into \
         another job in MERGE_BLOCKING_JOBS and add that job to main's required \
         status checks in the SAME change."
    );
}
