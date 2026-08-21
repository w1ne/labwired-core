// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Every board in the Arduino matrix must actually be RUN somewhere.
//!
//! `validation/arduino-matrix/boards.yaml` is a list of intentions. What turns
//! an intention into coverage is a CI lane that names the board. Those two
//! lists were only ever kept in agreement by hand, and the failure mode is
//! silent in the worst way: a board sits in boards.yaml, nothing runs it, and
//! the scoreboard's row for it simply never appears. Nobody sees a red cell —
//! they see no cell.
//!
//! This test closes that. A board is covered when either:
//!
//!   * `.github/workflows/core-arduino-matrix-smoke.yml` lists it in the job
//!     matrix — this repo compiles and runs it; or
//!   * the board declares `lane:` naming the workflow, in another repository,
//!     that owns its toolchain. BRD2709A is the case that forced this: its
//!     Arduino core is hand-written and lives in the labwired monorepo, so the
//!     compile cannot happen here (see the `external_compile:` note in
//!     boards.yaml).
//!
//! ⚠️ A `lane:` string is a claim this repo cannot verify — the workflow is in
//! another repository. What this test CAN enforce, and does, is that a board
//! makes the claim explicitly and that a board with no toolchain here cannot
//! quietly sit in the local matrix list pretending to be covered.

use std::path::PathBuf;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const BOARDS_YAML: &str = "validation/arduino-matrix/boards.yaml";
const WORKFLOW: &str = ".github/workflows/core-arduino-matrix-smoke.yml";

/// Board ids declared in boards.yaml, paired with their `lane:` if any.
fn declared_boards() -> Vec<(String, Option<String>, bool)> {
    let text = std::fs::read_to_string(root(BOARDS_YAML)).expect("boards.yaml");
    let mut out: Vec<(String, Option<String>, bool)> = Vec::new();
    let mut in_boards = false;
    for line in text.lines() {
        if line.starts_with("boards:") {
            in_boards = true;
            continue;
        }
        if line.starts_with("sketches:") {
            in_boards = false;
        }
        if !in_boards {
            continue;
        }
        if let Some(id) = line.strip_prefix("  - id: ") {
            out.push((id.trim().to_string(), None, false));
        } else if let Some(lane) = line.strip_prefix("    lane: ") {
            if let Some(last) = out.last_mut() {
                last.1 = Some(lane.trim().trim_matches('"').to_string());
            }
        } else if line.starts_with("    external_compile:") {
            if let Some(last) = out.last_mut() {
                last.2 = true;
            }
        }
    }
    out
}

/// Board ids listed in this repo's matrix workflow job matrix.
fn workflow_boards() -> Vec<String> {
    let text = std::fs::read_to_string(root(WORKFLOW)).expect("matrix workflow");
    let Some(start) = text.find("        board:\n") else {
        panic!("{WORKFLOW}: no `board:` job-matrix key — did the workflow change shape?");
    };
    text[start..]
        .lines()
        .skip(1)
        .take_while(|l| l.trim_start().starts_with("- "))
        .map(|l| l.trim().trim_start_matches("- ").to_string())
        .collect()
}

#[test]
fn every_matrix_board_has_a_lane() {
    let declared = declared_boards();
    assert!(
        declared.len() >= 16,
        "parsed only {} boards from {BOARDS_YAML} — the parser has lost the file's shape, \
         which would make this test vacuously green",
        declared.len()
    );
    let local = workflow_boards();
    assert!(
        local.len() >= 16,
        "parsed only {} boards from {WORKFLOW} — same vacuity risk",
        local.len()
    );

    let mut orphans = Vec::new();
    for (id, lane, _) in &declared {
        if local.contains(id) || lane.is_some() {
            continue;
        }
        orphans.push(id.clone());
    }
    assert!(
        orphans.is_empty(),
        "these boards are in {BOARDS_YAML} but nothing runs them: {orphans:?}\n\
         Add them to {WORKFLOW}'s job matrix, or give them a `lane:` naming the \
         workflow in another repo that does."
    );
}

/// The other direction: a board this repo's workflow names must exist in
/// boards.yaml, or the job spends a runner producing nothing.
#[test]
fn the_workflow_names_no_board_that_does_not_exist() {
    let declared: Vec<String> = declared_boards().into_iter().map(|(id, _, _)| id).collect();
    let unknown: Vec<String> = workflow_boards()
        .into_iter()
        .filter(|b| !declared.contains(b))
        .collect();
    assert!(
        unknown.is_empty(),
        "{WORKFLOW} names boards absent from {BOARDS_YAML}: {unknown:?}"
    );
}

/// A board that cannot compile in this repo must not be in this repo's job
/// matrix — the job would burn a runner and report `toolchain_missing`.
#[test]
fn an_external_compile_board_is_not_in_this_repos_job_matrix() {
    let local = workflow_boards();
    for (id, lane, external) in declared_boards() {
        if !external {
            continue;
        }
        assert!(
            !local.contains(&id),
            "{id} declares external_compile: — its toolchain is not in this repo, so \
             {WORKFLOW} cannot run it"
        );
        assert!(
            lane.is_some(),
            "{id} declares external_compile: but names no `lane:`. Without one, nothing \
             anywhere runs it and boards.yaml is claiming coverage it does not have."
        );
    }
}
