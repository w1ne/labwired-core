// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! BUS-PROOF MATRIX — per-chip UART / SPI / I²C evidence, gated.
//!
//! Why this file exists
//! ====================
//! "Does UART work on this chip?" had no answer you could read off anything.
//! The registers accept writes on every chip, the boot lanes are green on every
//! chip, and neither fact says a byte ever left the controller. The three
//! places that came closest each answer a different question:
//!
//! - `tier1` (`crates/cli/src/tier1.rs`) has `uart`/`spi`/`i2c` columns, but the
//!   `spi` and `i2c` cells are the FIXTURE's self-report about its own
//!   registers (`examples/tier1-fixture/*/src/main.rs`: write DR, poll BSY;
//!   START an absent address, require NACK). No device ever receives a byte.
//! - `validation/arduino-matrix` DOES put a real device on the wire (INA219 on
//!   `Wire`) and DOES read the verdict back over a real UART — but it needs
//!   PlatformIO, so it runs nightly, never on a PR.
//! - `board_coverage_ratchet.rs` ratchets reset/exec/display classes, and
//!   deliberately says nothing about buses.
//!
//! So this file records, per chip and per bus, what evidence EXISTS — in three
//! states, with the difference between them enforced rather than trusted:
//!
//! - `proven` — a test drives real firmware and asserts observable data moved:
//!   bytes at a UART sink, a modelled device answering with its real
//!   identity/payload, or a panel's own framebuffer painting. It must run
//!   somewhere, unguarded.
//! - `shallow` — evidence exists but does not carry that weight: it asserts
//!   register state only, or it moves real bytes but drives the controller from
//!   a harness instead of firmware, or it is `#[ignore]`d / self-skips on a
//!   missing fixture and therefore passes VACUOUSLY wherever it is invoked.
//! - `none` — nothing.
//!
//! ONE source of truth
//! ===================
//! `validation/bus_proof_matrix.json` is it. This file does NOT keep a second
//! copy of the verdicts (that is how a ratchet allowlist drifts from the thing
//! it ratchets); it keeps the RULES the JSON must satisfy.
//!
//! The two arms
//! ============
//! 1. STRUCTURAL (`bus_proof_matrix_is_complete_and_not_fabricated`) — needs no
//!    git and no fixtures, so it runs everywhere `cargo test --lib` runs, which
//!    is `pr-gate` on every PR:
//!      - the chip set is DERIVED from `configs/chips/*.yaml` (minus
//!        `ci-fixture-*`), exactly like `builtin_chip_self_contained.rs`. A new
//!        chip — including a new `BOARDS` chip in the superproject, which
//!        cannot ship without its `configs/chips/<id>.yaml` — has no row and
//!        FAILS. A row for a chip that no longer ships also fails.
//!      - a cell that is not `none` must cite a file that EXISTS and a test
//!        name that literally appears in it. A `proven` cell must additionally
//!        name the observable (`signal`) and the lane that runs it. You cannot
//!        make a cell green by typing `"proven"`.
//! 2. RATCHET (`bus_proof_matrix_never_regresses`) — compares every cell to the
//!    same file at `git merge-base HEAD origin/main`. `proven` may not become
//!    `shallow` or `none`; `shallow` may not become `none`. The baseline comes
//!    from OUTSIDE the change, so an author cannot move it by editing the JSON
//!    in the same commit. Under CI this is fatal if the baseline cannot be
//!    resolved — a ratchet that shrugs and passes is the bug it exists to stop.
//!
//!    "Cannot be resolved" is not the only way to get no gate, though, and the
//!    other way is quieter: a merge base resolved over a TRUNCATED or REWRITTEN
//!    parent chain (shallow clone, grafts, `refs/replace/*`, a blob-filtered
//!    partial clone) is not an error, it is a wrong commit — and when the matrix
//!    file is absent at that commit the arm below prints "new file" and returns
//!    green, in CI as well as locally. So `history_defect()` runs first and
//!    refuses, the same way `require_full_history()` does in
//!    scripts/generate_validation_status.py, where the identical defect rendered
//!    board dates four months wrong on a 112-commit shallow clone.
//!
//! `cell_ordering_detects_a_downgrade` proves the comparator is not vacuous by
//! watching it reject each downgrade direction, the way
//! `walk_starvation_contract::rule_c_detector_is_not_vacuous` does; the
//! `history_guard` module does the same for the history check, by watching it
//! fire on each condition in a synthetic repo.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// `validation/bus_proof_matrix.json`, relative to the workspace root.
pub const MATRIX_PATH: &str = "validation/bus_proof_matrix.json";

/// The three buses every row must answer for.
const BUSES: [&str; 3] = ["uart", "spi", "i2c"];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

// ─────────────────────────────────────────────────────────────────────────────
// The data
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Ordering matters: `None < Shallow < Proven`, and the ratchet is a `>=`.
    None,
    Shallow,
    Proven,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::None => "none",
            State::Shallow => "shallow",
            State::Proven => "proven",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cell {
    pub state: State,
    /// Repo-relative path of the test file backing this cell. Required unless
    /// `state == none`.
    #[serde(default)]
    pub file: String,
    /// A string that must literally appear in `file` — normally the test fn
    /// name, or the assertion line for a YAML-driven lab script.
    #[serde(default)]
    pub test: String,
    /// What is actually observed. Required for `proven`.
    #[serde(default)]
    pub signal: String,
    /// Where it runs. Required for `proven`; `"none"` is a legal value for a
    /// `shallow` cell and is exactly how a vacuous test is recorded.
    #[serde(default)]
    pub lane: String,
    /// Why this is not `proven`. Required for `shallow` and `none`.
    #[serde(default)]
    pub gap: String,
    /// Free-form caveat that does NOT change the verdict — e.g. "the proof is a
    /// one-byte write plus the device's ACK, not a payload readback". Optional
    /// everywhere; a `proven` cell may carry one, which is how a cell records
    /// the limits of its own proof without pretending to be `shallow`.
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Row {
    /// True when some board in the superproject's `packages/board-config`
    /// `BOARDS` declares `chip: "<this>"`. Informational: the gate derives its
    /// chip set from `configs/chips/`, which is a superset.
    #[serde(default)]
    pub in_boards: bool,
    #[serde(flatten)]
    pub buses: BTreeMap<String, Cell>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Matrix {
    pub chips: BTreeMap<String, Row>,
}

impl Matrix {
    fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("parse {MATRIX_PATH}: {e}"))
    }

    fn cells(&self) -> BTreeMap<(String, String), State> {
        let mut out = BTreeMap::new();
        for (chip, row) in &self.chips {
            for (bus, cell) in &row.buses {
                out.insert((chip.clone(), bus.clone()), cell.state);
            }
        }
        out
    }
}

fn load_committed() -> Matrix {
    let path = workspace_root().join(MATRIX_PATH);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    Matrix::parse(&text).unwrap_or_else(|e| panic!("{e}"))
}

/// Every chip descriptor we ship, derived — never hand-listed. Same rule as
/// `builtin_chip_self_contained::every_shipped_chip_file_is_offered_as_a_builtin`,
/// which is what keeps this set honest: a chip file that is not offered as a
/// built-in already fails there.
fn shipped_chips() -> BTreeSet<String> {
    let dir = workspace_root().join("configs/chips");
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(&dir).expect("read configs/chips") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|e| e != "yaml") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        // Internal harness fixtures, not silicon we offer.
        if stem.starts_with("ci-fixture-") {
            continue;
        }
        out.insert(stem);
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Arm 1 — structural: complete, well-formed, and not fabricated
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn bus_proof_matrix_is_complete_and_not_fabricated() {
    let matrix = load_committed();
    let shipped = shipped_chips();
    let root = workspace_root();
    let mut failures: Vec<String> = Vec::new();

    // Derived chip set — a new chip cannot slip in unrated.
    let rows: BTreeSet<String> = matrix.chips.keys().cloned().collect();
    let missing: Vec<&String> = shipped.difference(&rows).collect();
    if !missing.is_empty() {
        failures.push(format!(
            "these chips ship (configs/chips/<id>.yaml) but have NO row in {MATRIX_PATH}: \
             {missing:?}.\n  Add a row. A chip whose UART/SPI/I²C evidence nobody has \
             written down is a chip nobody has checked — `none` is a legal, honest answer."
        ));
    }
    let extra: Vec<&String> = rows.difference(&shipped).collect();
    if !extra.is_empty() {
        failures.push(format!(
            "{MATRIX_PATH} has rows for chips that no longer ship: {extra:?}. Delete them."
        ));
    }

    for (chip, row) in &matrix.chips {
        let present: BTreeSet<&str> = row.buses.keys().map(String::as_str).collect();
        for bus in BUSES {
            if !present.contains(bus) {
                failures.push(format!("{chip}: missing `{bus}` cell"));
            }
        }
        for bus in present {
            if !BUSES.contains(&bus) {
                failures.push(format!(
                    "{chip}: unknown bus `{bus}` (expected exactly {BUSES:?})"
                ));
            }
        }

        for (bus, cell) in &row.buses {
            let at = format!("{chip}/{bus}");
            match cell.state {
                State::None => {
                    if cell.gap.trim().is_empty() {
                        failures.push(format!("{at}: `none` must say why in `gap`"));
                    }
                    if !cell.file.trim().is_empty() {
                        failures.push(format!(
                            "{at}: `none` cites {}: if a test exists this is not `none`",
                            cell.file
                        ));
                    }
                    if !cell.note.trim().is_empty() {
                        failures.push(format!(
                            "{at}: `none` has nothing to caveat — put it in `gap`"
                        ));
                    }
                }
                State::Shallow | State::Proven => {
                    // Anti-fabrication: the cited evidence must exist, and the
                    // cited test name must literally be in it.
                    if cell.file.trim().is_empty() || cell.test.trim().is_empty() {
                        failures.push(format!(
                            "{at}: `{}` must cite both `file` and `test`",
                            cell.state.label()
                        ));
                        continue;
                    }
                    let path = root.join(&cell.file);
                    match std::fs::read_to_string(&path) {
                        Err(e) => failures.push(format!(
                            "{at}: cites `{}` which cannot be read ({e}). \
                             A cell may not cite evidence that is not there.",
                            cell.file
                        )),
                        Ok(text) => {
                            if !text.contains(&cell.test) {
                                failures.push(format!(
                                    "{at}: `{}` does not appear in `{}`. Either the test was \
                                     renamed or deleted (fix the row, or downgrade the cell) \
                                     or the citation was invented.",
                                    cell.test, cell.file
                                ));
                            }
                        }
                    }
                    if cell.state == State::Proven {
                        if cell.signal.trim().is_empty() {
                            failures.push(format!(
                                "{at}: `proven` must name the observable in `signal` \
                                 (bytes at a sink / a device's own payload / painted pixels). \
                                 'the registers looked right' is `shallow`."
                            ));
                        }
                        if cell.lane.trim().is_empty() || cell.lane == "none" {
                            failures.push(format!(
                                "{at}: `proven` must name the lane that RUNS it. A test that \
                                 runs nowhere proves nothing — that is `shallow`."
                            ));
                        }
                        if !cell.gap.trim().is_empty() {
                            failures.push(format!(
                                "{at}: `proven` must not carry a `gap` (it has none)"
                            ));
                        }
                    } else if cell.gap.trim().is_empty() {
                        failures.push(format!(
                            "{at}: `shallow` must say in `gap` what it fails to prove"
                        ));
                    }
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "\nBUS-PROOF MATRIX ({MATRIX_PATH}) is not honest:\n\n  {}\n",
        failures.join("\n  ")
    );

    // Print the table. `--nocapture` in a lane turns this into the evidence that
    // the gate READ something, rather than merely existing — the same reason
    // `pr-scheduler-observable` prints `sink_bytes=N`.
    eprintln!("bus-proof matrix ({} chips)", matrix.chips.len());
    eprintln!(
        "  {:<16} {:<7} {:<8} {:<8} {:<8}",
        "chip", "BOARDS", "uart", "spi", "i2c"
    );
    let mut proven = 0usize;
    for (chip, row) in &matrix.chips {
        let get = |b: &str| row.buses[b].state.label();
        eprintln!(
            "  {:<16} {:<7} {:<8} {:<8} {:<8}",
            chip,
            if row.in_boards { "yes" } else { "-" },
            get("uart"),
            get("spi"),
            get("i2c")
        );
        proven += row
            .buses
            .values()
            .filter(|c| c.state == State::Proven)
            .count();
    }
    eprintln!(
        "  proven {}/{} cells",
        proven,
        matrix.chips.len() * BUSES.len()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Arm 2 — the ratchet: cells improve, never regress
// ─────────────────────────────────────────────────────────────────────────────

/// The comparison itself, factored out so it can be tested against synthetic
/// input rather than only against whatever the tree happens to contain.
fn downgrades(
    baseline: &BTreeMap<(String, String), State>,
    live: &BTreeMap<(String, String), State>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (cell, was) in baseline {
        match live.get(cell) {
            None => out.push(format!(
                "{}/{}: {} -> row DELETED. Deleting a cell is a downgrade.",
                cell.0,
                cell.1,
                was.label()
            )),
            Some(now) if now < was => out.push(format!(
                "{}/{}: {} -> {}",
                cell.0,
                cell.1,
                was.label(),
                now.label()
            )),
            Some(_) => {}
        }
    }
    out
}

/// `git show <rev>:<path>`, or `None` when the rev/path is not there.
fn git_show(rev: &str, path: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["show", &format!("{rev}:{path}")])
        .current_dir(workspace_root())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn merge_base_with_trunk() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["merge-base", "HEAD", "origin/main"])
        .current_dir(workspace_root())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// stdout of a read-only git command in `root`, or `None` on a nonzero exit.
/// Empty output and "git failed" are different answers: the callers below rely
/// on telling them apart.
///
/// `root` is a parameter rather than always `workspace_root()` so the guard
/// below can be pointed at a synthetic checkout and PROVEN to fire. A detector
/// that has never been observed detecting is the same kind of unearned
/// confidence as a `proven` cell nobody ran.
fn git_out(root: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Why this checkout cannot be trusted to resolve a ratchet baseline, or `None`.
///
/// WHY THIS EXISTS AT ALL
///     The ratchet's whole claim is that the baseline comes from OUTSIDE the
///     change under test. That claim rests on `git merge-base HEAD origin/main`
///     naming the real branch point, and every condition below is a way for the
///     commit walk to answer from a rewritten or truncated graph instead —
///     confidently, with no error.
///
///     The damage is not a red build. Look at the arm below: when
///     `git show <base>:<matrix>` comes back empty the test PRINTS "new file"
///     and returns green, and that path is not gated on `CI` the way the
///     unresolvable-merge-base path is. So a merge base resolved against a
///     rewritten parent chain — one that predates this file, or that never had
///     it — turns the ratchet into a silent pass in CI. A downgrade from
///     `proven` to `none` would merge with the gate reporting nothing at all.
///     That skip is only honest once the history behind it is known-good, which
///     is what this function establishes.
///
/// WHY IT IS DUPLICATED RATHER THAN SHARED
///     `history_defect()` in scripts/generate_validation_status.py is the same
///     check for the same reason (its board dates read the graft boundary
///     instead of the real last-touch commit — `ci-fixture-riscv` came out four
///     months wrong), and crates/cli/src/tier1.rs guards its own merge-base
///     ratchet likewise. Three call sites, three languages/crates, and no home
///     that all three can import: labwired-core is the engine and compiles to
///     wasm, so `std::process::Command` plumbing must not leave `cfg(test)`
///     here. Duplicating ~20 lines of plumbing beats a crate whose only job is
///     to hold them.
fn history_defect(root: &std::path::Path) -> Option<String> {
    // Not a repository at all: `merge-base` fails, which the caller already
    // handles — but say so precisely rather than as "no origin/main".
    if git_out(root, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return Some("not a git checkout — there is no history to resolve a baseline in".into());
    }

    // The common case, and the one CI can create by lowering fetch-depth.
    if git_out(root, &["rev-parse", "--is-shallow-repository"]).as_deref() == Some("true") {
        return Some("shallow clone — history is truncated at the graft boundary".into());
    }

    // `git replace` refs and the legacy .git/info/grafts file rewrite the parent
    // chain the merge-base walk follows, so it can terminate at a synthetic
    // boundary exactly as a shallow clone does — except that `--is-shallow-
    // repository` says false and merge-base SUCCEEDS, returning a commit that is
    // a common ancestor of a graph nobody pushed. Rare; the wrongness is silent.
    if let Some(grafts) = git_out(root, &["rev-parse", "--git-path", "info/grafts"]) {
        // --git-path answers relative to git's cwd, i.e. the workspace root;
        // joining is a no-op when it comes back absolute (linked worktrees).
        if root.join(&grafts).exists() {
            return Some(format!(
                "grafted history (`{grafts}` exists) — the parent chain is rewritten"
            ));
        }
    }
    if git_out(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/replace/"],
    )
    .is_some_and(|s| !s.is_empty())
    {
        return Some(
            "replace refs present (refs/replace/*) — the parent chain is rewritten".into(),
        );
    }

    // Partial clones. Note this is STRICTER than the identically-named check in
    // generate_validation_status.py, deliberately: that script only walks commits
    // and diffs trees, so `blob:none` leaves its answers exact and is allowed.
    // This one reads a BLOB — `git show <rev>:validation/bus_proof_matrix.json` —
    // so `blob:none` is precisely the filter that bites. Offline, or against an
    // unreachable promisor, the read returns nothing and the arm takes the "new
    // file" skip above. Any filter is unusable here.
    let configured = git_out(
        root,
        &["config", "--get-regexp", r"^remote\..*\.partialclonefilter"],
    );
    if let Some(text) = configured {
        let filters: Vec<&str> = text
            .lines()
            // `remote.origin.partialclonefilter blob:none` — key, space, spec.
            .filter_map(|l| l.split_once(' ').map(|(_, spec)| spec.trim()))
            .collect();
        if !filters.is_empty() {
            return Some(format!(
                "partial clone ({}) — the baseline matrix blob is not local",
                filters.join(", ")
            ));
        }
    }
    None
}

/// Refuse to draw a ratchet verdict from a history that cannot answer, and say
/// which condition it was. Returns `true` when the caller should stop.
///
/// Fatal under `CI` for the same reason an unresolvable merge base is: a ratchet
/// that shrugs and passes is the bug it exists to stop. Locally it degrades to a
/// loud skip, because a contributor on a `--depth` clone should still get the
/// structural arm rather than a failure they cannot act on.
fn history_is_unusable(what: &str) -> bool {
    let Some(defect) = history_defect(&workspace_root()) else {
        return false;
    };
    assert!(
        std::env::var("CI").is_err(),
        "bus-proof ratchet: {what} cannot be trusted — {defect}. The lane needs \
         `actions/checkout` with `fetch-depth: 0` and an `origin/main` ref. A bounded \
         `--depth=<n>` is not a fix: it only moves the boundary, and merge-base then \
         answers from a graph nobody pushed. Refusing to pass without a real baseline."
    );
    eprintln!("SKIP(baseline): {defect}. The structural arm still ran; the ratchet arm did not.");
    true
}

#[test]
fn bus_proof_matrix_never_regresses() {
    let live = load_committed().cells();

    // `CI` is set by GitHub Actions. There, an unresolvable baseline is fatal:
    // the whole point is that the author cannot move it.
    let required = std::env::var("CI").is_ok();

    // BEFORE resolving anything: a merge base computed over a truncated or
    // rewritten parent chain is not "no baseline", it is a WRONG baseline, and
    // the "new file" skip below would then pass this arm in silence.
    if history_is_unusable("the merge base with origin/main") {
        return;
    }

    let Some(base_rev) = merge_base_with_trunk() else {
        assert!(
            !required,
            "bus-proof ratchet: cannot resolve `git merge-base HEAD origin/main`. \
             The lane needs fetch-depth: 0 and an origin/main ref. Refusing to pass \
             without a baseline."
        );
        eprintln!(
            "SKIP(baseline): no `git merge-base HEAD origin/main` in this checkout. \
             The structural arm still ran; the ratchet arm did not."
        );
        return;
    };

    let Some(text) = git_show(&base_rev, MATRIX_PATH) else {
        // The very first commit that adds the file has no baseline, by
        // construction. Every later one does — and this skip is trustworthy
        // ONLY because history_is_unusable() ran above: without it, "absent at
        // the merge base" and "absent because the merge base is fiction" are
        // the same observation, and this line returns green for both.
        eprintln!("SKIP(baseline): {MATRIX_PATH} does not exist at {base_rev} (new file).");
        return;
    };

    let baseline = Matrix::parse(&text)
        .unwrap_or_else(|e| panic!("bus-proof ratchet: baseline at {base_rev} is unparsable: {e}"))
        .cells();

    eprintln!(
        "bus-proof baseline: {base_rev} ({} cells) vs live ({} cells)",
        baseline.len(),
        live.len()
    );

    let bad = downgrades(&baseline, &live);
    assert!(
        bad.is_empty(),
        "\nBUS-PROOF RATCHET: {} cell(s) got WORSE than the merge base:\n\n  {}\n\n\
         A cell may improve or hold. If a test really was deleted or made vacuous, \
         that is a coverage regression to argue for in review — not a JSON edit to \
         slip past a gate.\n",
        bad.len(),
        bad.join("\n  ")
    );
}

/// The comparator must actually reject downgrades. A ratchet whose detector is
/// broken is a green gate that guards nothing.
#[test]
fn cell_ordering_detects_a_downgrade() {
    assert!(State::None < State::Shallow && State::Shallow < State::Proven);

    let cell = |c: &str, b: &str| (c.to_string(), b.to_string());
    let base: BTreeMap<_, _> = [
        (cell("esp32", "uart"), State::Proven),
        (cell("esp32", "spi"), State::Shallow),
        (cell("esp32", "i2c"), State::None),
    ]
    .into_iter()
    .collect();

    // Holding, and improving, are fine.
    assert!(downgrades(&base, &base).is_empty());
    let better: BTreeMap<_, _> = base.keys().map(|k| (k.clone(), State::Proven)).collect();
    assert!(downgrades(&base, &better).is_empty());

    // Every downgrade direction is caught.
    for (bus, to) in [
        ("uart", State::Shallow),
        ("uart", State::None),
        ("spi", State::None),
    ] {
        let mut worse = base.clone();
        worse.insert(cell("esp32", bus), to);
        let hits = downgrades(&base, &worse);
        assert_eq!(
            hits.len(),
            1,
            "downgrade esp32/{bus} -> {} missed",
            to.label()
        );
        assert!(hits[0].starts_with(&format!("esp32/{bus}: ")), "{hits:?}");
    }

    // Deleting a cell is a downgrade, not a way out.
    let mut deleted = base.clone();
    deleted.remove(&cell("esp32", "uart"));
    assert_eq!(downgrades(&base, &deleted).len(), 1);
}

/// The ratchet's git plumbing must work BEFORE it has anything to compare.
///
/// `bus_proof_matrix_never_regresses` cannot bite until this file exists on
/// `main`: until then `git_show` legitimately returns `None` and the arm skips.
/// That is a window in which a broken `merge-base` or `git show` would look
/// exactly like "no baseline yet", so this test closes it by resolving the
/// merge base and reading a file that has been there all along.
#[test]
fn baseline_plumbing_can_read_the_merge_base() {
    if history_is_unusable("the merge base with origin/main") {
        return;
    }
    let Some(base) = merge_base_with_trunk() else {
        assert!(
            std::env::var("CI").is_err(),
            "no `git merge-base HEAD origin/main` under CI — the lane needs \
             fetch-depth: 0 and an origin/main ref"
        );
        eprintln!("SKIP: shallow/detached checkout with no origin/main");
        return;
    };
    assert_eq!(base.len(), 40, "merge base is not a full sha: {base:?}");
    let anchor = git_show(&base, "Cargo.toml")
        .unwrap_or_else(|| panic!("`git show {base}:Cargo.toml` returned nothing"));
    assert!(
        anchor.contains("[workspace]"),
        "read something from the merge base, but it is not the workspace manifest"
    );
    // And a path that has never existed must be reported as absent, not as "".
    assert!(git_show(&base, "validation/definitely_not_a_file.json").is_none());
}

/// The history guard must actually fire, or it is decoration on a silent pass.
///
/// Same reasoning as `cell_ordering_detects_a_downgrade`: this file's whole
/// method is to prove its detectors reject, rather than to trust that they
/// would. A guard nobody has watched trigger is exactly the "passes VACUOUSLY"
/// state the matrix records as `shallow` for everyone else's tests.
#[cfg(test)]
mod history_guard {
    use super::history_defect;
    use std::path::{Path, PathBuf};

    /// Run git in `dir` and return trimmed stdout, panicking on failure.
    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// A throwaway repository with identity configured, so the tests that need
    /// real commits (the replace ref must point at objects that EXIST — git
    /// silently ignores a broken ref, which would make that test vacuous) can
    /// make them.
    fn scratch_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bus-proof-history-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "t@example.invalid"]);
        git(&dir, &["config", "user.name", "t"]);
        dir
    }

    /// Two commits, returning (HEAD, HEAD~1).
    fn two_commits(dir: &Path) -> (String, String) {
        std::fs::write(dir.join("f"), "a").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", "one", "--no-verify"]);
        std::fs::write(dir.join("f"), "b").unwrap();
        git(dir, &["commit", "-qam", "two", "--no-verify"]);
        (
            git(dir, &["rev-parse", "HEAD"]),
            git(dir, &["rev-parse", "HEAD~1"]),
        )
    }

    fn defect_of(dir: &Path) -> String {
        history_defect(dir).unwrap_or_else(|| {
            panic!(
                "history_defect() saw nothing wrong with {} — the guard is vacuous",
                dir.display()
            )
        })
    }

    #[test]
    fn a_fresh_full_repo_is_not_flagged() {
        // The other half of "not vacuous": a guard that flags everything would
        // turn every local `cargo test` into a failure and get deleted.
        let dir = scratch_repo("clean");
        assert!(history_defect(&dir).is_none(), "{:?}", history_defect(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shallow_clone_is_flagged() {
        let dir = scratch_repo("shallow");
        // Exactly what `git clone --depth` / `actions/checkout` with a bounded
        // fetch-depth leaves behind; `rev-parse --is-shallow-repository` keys
        // on this file and nothing else.
        std::fs::write(dir.join(".git/shallow"), "").unwrap();
        let defect = defect_of(&dir);
        assert!(defect.contains("shallow"), "{defect}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_grafted_history_is_flagged() {
        // The case a shallow check alone misses: `--is-shallow-repository` says
        // false, `merge-base` SUCCEEDS, and it answers from a parent chain that
        // was rewritten locally. Wrong, and silent.
        let dir = scratch_repo("grafts");
        std::fs::create_dir_all(dir.join(".git/info")).unwrap();
        std::fs::write(dir.join(".git/info/grafts"), "").unwrap();
        let defect = defect_of(&dir);
        assert!(defect.contains("graft"), "{defect}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_replace_ref_is_flagged() {
        let dir = scratch_repo("replace");
        let (head, parent) = two_commits(&dir);
        // A REAL replace mapping: git drops a ref whose target is missing
        // ("ignoring broken ref"), so a placeholder sha here would make this
        // test pass for the wrong reason and prove nothing.
        git(&dir, &["replace", &head, &parent]);
        let defect = defect_of(&dir);
        assert!(defect.contains("replace refs"), "{defect}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_partial_clone_is_flagged() {
        // Stricter here than in generate_validation_status.py on purpose: this
        // ratchet reads a BLOB out of the baseline commit, so `blob:none` — the
        // filter that script explicitly allows — is the one that breaks it.
        let dir = scratch_repo("partial");
        for args in [
            [
                "config",
                "remote.origin.url",
                "https://example.invalid/x.git",
            ],
            ["config", "remote.origin.partialclonefilter", "blob:none"],
        ] {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(out.status.success());
        }
        let defect = defect_of(&dir);
        assert!(
            defect.contains("partial clone") && defect.contains("blob:none"),
            "{defect}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_that_is_not_a_repo_is_flagged() {
        let dir = std::env::temp_dir().join(format!("bus-proof-nogit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let defect = defect_of(&dir);
        assert!(defect.contains("not a git checkout"), "{defect}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
