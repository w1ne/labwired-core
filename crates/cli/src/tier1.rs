// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Tier-1 chip × peripheral validation matrix.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One cell's status. `Na` = chip YAML declares no peripheral of this class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellStatus {
    Pass,
    Partial,
    Blocked,
    Na,
    Unrecorded,
}

impl CellStatus {
    /// Snapshot vocabulary — must stay in sync with the serde snake_case names.
    pub fn as_str(self) -> &'static str {
        match self {
            CellStatus::Pass => "pass",
            CellStatus::Partial => "partial",
            CellStatus::Blocked => "blocked",
            CellStatus::Na => "na",
            CellStatus::Unrecorded => "unrecorded",
        }
    }
}

/// A cell with its evidence link (CI run that produced it; None until CI stamps it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub status: CellStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_url: Option<String>,
}

/// chip → class → cell. BTreeMaps keep JSON output deterministic (sorted keys).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier1Matrix(pub BTreeMap<String, BTreeMap<String, Cell>>);

/// The standard classes every chip's row carries: the six bring-up rubric
/// classes plus the typical MCU peripheral set. Classes a chip doesn't declare
/// render `na`; classes a fixture hasn't attempted yet render `unrecorded`.
pub const RUBRIC_CLASSES: &[&str] = &[
    "clock", "gpio", "uart", "timer", "dma", "irq", // bring-up rubric
    "i2c", "spi", "adc", "pwm", "wdt", "rtc", // typical peripherals
];

/// Parsed `TIER1` protocol from a UART capture.
#[derive(Debug, Default)]
pub struct ParsedTier1 {
    /// class → status from explicit `TIER1 <class> PASS|FAIL` lines.
    /// Repeated reports for a class take the last occurrence — supports
    /// fixture-internal retries. Class tokens are case-sensitive: `TIER1 GPIO
    /// PASS` records a class `GPIO` that no standard row consumes.
    pub classes: BTreeMap<String, CellStatus>,
    /// `TIER1 done` seen — the fixture completed its sequence.
    pub done: bool,
}

/// Parse `TIER1 <class> PASS|FAIL[ code=..]` lines + `TIER1 done` out of a raw
/// UART byte capture. Non-UTF8 and unrelated lines are skipped; malformed
/// `TIER1` lines are ignored (never fatal — boot noise is expected). Leading and
/// trailing whitespace on each token is normalised by `split_whitespace`; CRLF
/// line endings are handled by `lines()`.
/// Printed-class aliases: fixtures may report a chip-specific peripheral name
/// that maps onto a standard column (the ESP32-S3 fixture prints `mcpwm`, that
/// chip's PWM block). Applied at parse time so committed fixture blobs keep
/// working as columns standardize.
const CLASS_ALIASES: &[(&str, &str)] = &[("mcpwm", "pwm"), ("ledc", "pwm")];

fn canonical_class(class: &str) -> String {
    for (alias, std) in CLASS_ALIASES {
        if class == *alias {
            return (*std).to_string();
        }
    }
    class.to_string()
}

pub fn parse_tier1_uart(uart: &[u8]) -> ParsedTier1 {
    let mut out = ParsedTier1::default();
    for line in String::from_utf8_lossy(uart).lines() {
        let mut it = line.split_whitespace();
        if it.next() != Some("TIER1") {
            continue;
        }
        match (it.next(), it.next()) {
            (Some("done"), _) => out.done = true,
            (Some(class), Some("PASS")) => {
                out.classes.insert(canonical_class(class), CellStatus::Pass);
            }
            (Some(class), Some("FAIL")) => {
                out.classes
                    .insert(canonical_class(class), CellStatus::Blocked);
            }
            _ => {} // malformed TIER1 line — ignore
        }
    }
    out
}

impl ParsedTier1 {
    /// Resolve a full row over `classes`. Rules (spec §2 conventions):
    ///
    /// - If the fixture explicitly reported `uart` (a `TIER1 uart PASS|FAIL`
    ///   line), that explicit status wins, subject to the same done-degradation
    ///   rule as every other class (explicit Pass without done → Partial).
    /// - Otherwise `uart` is Pass iff `done` was seen — receiving a `TIER1
    ///   done` line over UART is itself the proof of a working UART channel.
    ///   `!classes.is_empty()` is **not** required.
    /// - Missing `done` degrades every reported Pass to Partial (hung
    ///   mid-sequence), and classes never reported are Blocked (the fixture
    ///   hung before reaching them).
    /// - With `done` seen, classes never reported are Unrecorded — the fixture
    ///   simply doesn't attempt them yet; no claim either way.
    pub fn resolve_row(&self, classes: &[&str]) -> BTreeMap<String, Cell> {
        let mut row = BTreeMap::new();
        for &class in classes {
            let status = if class == "uart" {
                match self.classes.get("uart") {
                    // Explicit uart verdict from the fixture — honour it, same
                    // done-degradation as every other class.
                    Some(CellStatus::Pass) if !self.done => CellStatus::Partial,
                    Some(s) => *s,
                    // No explicit uart line: done alone proves UART is alive.
                    None if self.done => CellStatus::Pass,
                    None => CellStatus::Blocked,
                }
            } else {
                match self.classes.get(class) {
                    Some(CellStatus::Pass) if !self.done => CellStatus::Partial,
                    Some(s) => *s,
                    // Not attempted by this fixture: no claim either way. The
                    // ratchet flags pass->unrecorded if a check is removed.
                    None if self.done => CellStatus::Unrecorded,
                    None => CellStatus::Blocked, // hung before reaching it
                }
            };
            row.insert(
                class.to_string(),
                Cell {
                    status,
                    run_url: None,
                },
            );
        }
        row
    }
}

/// peripheral-id substring → tier1 class. First match wins. Order-sensitive
/// pair: `"_pwm"` must precede `"tim"` — STM32 advanced-control timers declare
/// the pwm class via an `_pwm` id suffix (e.g. `tim1_pwm`), which would
/// otherwise be swallowed by the `tim`→timer marker. (The timer class itself
/// comes from the plain `timN` instances.)
const CLASS_MARKERS: &[(&str, &str)] = &[
    ("_pwm", "pwm"),
    ("uart", "uart"),
    ("usart", "uart"),      // STM32 naming: usart1 does not substring-match "uart"
    ("usb_serial", "uart"), // S3 console can be USB-Serial-JTAG
    ("gpio", "gpio"),
    ("sio", "gpio"), // RP2040 single-cycle IO block (id/type `sio`) is its GPIO
    ("timg", "timer"),
    ("systimer", "timer"),
    ("tim", "timer"),
    ("gdma", "dma"),
    ("dma", "dma"),
    ("intmatrix", "irq"),
    ("interrupt", "irq"),
    ("nvic", "irq"),
    ("rcc", "clock"),
    ("clk", "clock"),
    ("rtc_cntl", "clock"),
    ("clock", "clock"), // nRF CLOCK block (id `clock` / type `nrf_clock`)
    ("system", "clock"),
    ("i2c", "i2c"),
    ("twi", "i2c"), // Nordic naming: TWI/TWIM/TWIS are the I²C blocks
    ("spi", "spi"),
    ("sar_adc", "adc"),
    ("adc", "adc"),
    ("mcpwm", "pwm"),
    ("ledc", "pwm"),
    ("pwm", "pwm"),
    ("iwdg", "wdt"),
    ("wwdg", "wdt"),
    ("wdt", "wdt"),
    // RP2040 names its block `watchdog` outright, which contains none of the
    // abbreviations above — without this marker the class read `na` even with a
    // behavioural model wired up.
    ("watchdog", "wdt"),
    // Deliberately "fdcan", not "can": bxCAN instances (stm32f103
    // `bxcan1`, stm32l476 `can1`) must not declare the class until
    // their fixtures actually check it.
    ("fdcan", "can"),
    // NOTE: "rtc_cntl" -> clock is matched FIRST (listed above); bare "rtc"
    // ids map to the rtc class.
    ("rtc", "rtc"),
    ("rmt", "rmt"),
];

#[derive(Deserialize)]
struct ChipYamlPeripheral {
    id: String,
    #[serde(default)]
    r#type: String,
}

#[derive(Deserialize)]
struct ChipYamlDoc {
    #[serde(default)]
    peripherals: Vec<ChipYamlPeripheral>,
}

/// Which tier1 classes a chip YAML declares, by peripheral heuristics.
///
/// Both the instance `id` and the model `type` are matched: instance ids
/// follow chip-vendor naming that the marker table can't enumerate (`twi1`
/// is the nRF I²C, `clock` the nRF CLOCK), while the `type` field carries
/// the family-qualified model name (`nrf52840_i2c`, `nrf_clock`) that the
/// markers reliably hit. Matching only ids made whole modeled subsystems
/// render as "not modeled" in the public matrix.
pub fn declared_classes_from_yaml(yaml: &str) -> Result<BTreeSet<String>, String> {
    let doc: ChipYamlDoc = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
    let mut classes = BTreeSet::new();
    for p in &doc.peripherals {
        for name in [p.id.to_lowercase(), p.r#type.to_lowercase()] {
            for (marker, class) in CLASS_MARKERS {
                if name.contains(marker) {
                    classes.insert(class.to_string());
                    break;
                }
            }
        }
    }
    Ok(classes)
}

/// Cells whose class is not declared by the chip become `Na`. This deliberately
/// downgrades even Pass cells — heuristic misses surface as `pass -> na` in the
/// ratchet diff rather than silently shadow-passing.
pub fn apply_na(row: &mut BTreeMap<String, Cell>, declared: &BTreeSet<String>) {
    for (class, cell) in row.iter_mut() {
        if !declared.contains(class) {
            cell.status = CellStatus::Na;
            cell.run_url = None;
        }
    }
}

/// Chips that the snapshot records with at least one `pass` cell but which the
/// live run skipped (fixture missing). A deleted fixture must not silently
/// disarm the ratchet — these are reported as regressions by the gate.
pub fn skipped_chips_with_recorded_passes(
    snapshot: &Tier1Matrix,
    skipped: &[String],
) -> Vec<String> {
    skipped
        .iter()
        .filter(|chip| {
            snapshot
                .0
                .get(chip.as_str())
                .is_some_and(|row| row.values().any(|c| c.status == CellStatus::Pass))
        })
        .cloned()
        .collect()
}

/// Cells recorded `Pass` in the snapshot must still pass live. Everything else
/// (partial/blocked/na/unrecorded, chips missing from the live run) moves freely.
pub fn ratchet_regressions(snapshot: &Tier1Matrix, live: &Tier1Matrix) -> Vec<String> {
    let mut out = Vec::new();
    for (chip, row) in &snapshot.0 {
        for (class, snap_cell) in row {
            if snap_cell.status != CellStatus::Pass {
                continue;
            }
            let live_status = live
                .0
                .get(chip)
                .and_then(|r| r.get(class))
                .map(|c| c.status);
            match live_status {
                Some(CellStatus::Pass) | None => {} // None = chip not exercised in this run
                Some(s) => out.push(format!("{chip}/{class}: pass -> {}", s.as_str())),
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Merge-base baseline ratchet
//
// WHY THIS EXISTS. `ratchet_regressions` compares the COMMITTED snapshot against
// a live run. Any change that regenerates the snapshot makes those two equal by
// construction, so the ratchet is green no matter how far a row drops. That is
// not hypothetical: the ESP32-C3 row went from 8 `pass` cells to 0 across two
// onboarding PRs (STM32H735 2026-07-21, STM32F401 2026-07-28) that touched no
// C3 file at all — they simply regenerated the snapshot on machines where the
// C3 fixture had gone stale. CI was green the whole way.
//
// The fix is to compare the live run against a baseline the author of the
// change cannot move: the matrix as it is recorded OUTSIDE the change.
// ─────────────────────────────────────────────────────────────────────────────

/// Repo-relative path of the committed matrix snapshot.
pub const MATRIX_PATH: &str = "docs/coverage/tier1-matrix.json";

/// Repo-relative path of the acknowledgement file for intentional drops.
pub const RATCHET_ACK_PATH: &str = "docs/coverage/tier1-ratchet-ack.yaml";

/// Ref the baseline is taken against when the env override is unset.
pub const DEFAULT_BASELINE_REF: &str = "origin/main";

/// Override for [`DEFAULT_BASELINE_REF`] — forks, release branches, and the
/// gate's own proof runs. It selects WHICH ref plays the role of the trunk; it
/// can never switch the gate off, and an unresolvable ref is a hard error.
pub const BASELINE_REF_ENV: &str = "LABWIRED_TIER1_BASELINE_REF";

/// Minimum length of an ack `reason`. An acknowledgement has to say something;
/// "wip" is silence with extra steps.
const MIN_ACK_REASON_LEN: usize = 30;

/// A cell that dropped below the baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Regression {
    pub chip: String,
    pub class: String,
    /// What the live run reported (the baseline side is always `pass`).
    pub to: CellStatus,
}

impl Regression {
    /// `chip/class` — the key an ack entry names.
    pub fn cell(&self) -> String {
        format!("{}/{}", self.chip, self.class)
    }
}

impl std::fmt::Display for Regression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: pass -> {}", self.cell(), self.to.as_str())
    }
}

/// Cells recorded `Pass` in `baseline` that are not `Pass` in `live`.
///
/// Deliberately identical in spirit to [`ratchet_regressions`], but structured
/// so the ack matcher can key on chip+class. A chip absent from `live` is not
/// reported here — that case is [`skipped_chips_with_recorded_passes`], which
/// the gate runs against the baseline too.
pub fn baseline_regressions(baseline: &Tier1Matrix, live: &Tier1Matrix) -> Vec<Regression> {
    let mut out = Vec::new();
    for (chip, row) in &baseline.0 {
        for (class, base_cell) in row {
            if base_cell.status != CellStatus::Pass {
                continue;
            }
            let Some(live_cell) = live.0.get(chip).and_then(|r| r.get(class)) else {
                continue; // chip/class not exercised in this run
            };
            if live_cell.status != CellStatus::Pass {
                out.push(Regression {
                    chip: chip.clone(),
                    class: class.clone(),
                    to: live_cell.status,
                });
            }
        }
    }
    out
}

/// One acknowledged, intentional drop. Named explicitly — silence is never
/// sufficient. Mirrors the `drift_ack` convention in `validation/manifest.yaml`:
/// one trailing key that has to COVER the specific observed drift, plus prose.
#[derive(Debug, Clone, Deserialize)]
pub struct RatchetAck {
    /// `chip/class`, e.g. `esp32c3/i2c`.
    pub cell: String,
    /// The status the live run is expected to report. An ack for `partial` does
    /// NOT cover a later slide to `blocked`.
    pub to: String,
    /// Why the drop is intentional. Must be substantive.
    pub reason: String,
}

#[derive(Debug, Default, Deserialize)]
struct RatchetAckDoc {
    #[serde(default)]
    acks: Vec<RatchetAck>,
}

/// Load the ack file. A missing file means "no acknowledgements" (the normal
/// state); a malformed one is a hard error — a gate must never read a broken
/// ack file as an empty one.
pub fn load_ratchet_acks(root: &Path) -> Result<Vec<RatchetAck>, String> {
    let path = root.join(RATCHET_ACK_PATH);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: RatchetAckDoc =
        serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(doc.acks)
}

/// Resolve the baseline commit for the ratchet, as a `(description, commit)`
/// pair. See [`resolve_baseline_matrix`] for the rule.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The ref the baseline is taken against.
pub fn baseline_ref() -> String {
    std::env::var(BASELINE_REF_ENV).unwrap_or_else(|_| DEFAULT_BASELINE_REF.to_string())
}

/// Resolve the immovable baseline matrix, plus a human description of where it
/// came from.
///
/// Rule, in one sentence: **compare against the newest recorded matrix that the
/// change under test did not write.**
///
/// * `merge-base(HEAD, baseline_ref) != HEAD` — a branch. The baseline is the
///   matrix at the merge base, i.e. what the trunk says today. Regenerating the
///   snapshot on the branch cannot move it.
/// * `merge-base == HEAD` — we ARE on (or at) the trunk. The baseline is the
///   matrix as of the commit before the newest one that touched the file, so a
///   commit that lowers the matrix is measured against the matrix it replaced.
///   If HEAD did not touch the file there is nothing new to ratchet and the
///   baseline is simply the current recorded matrix (the committed-vs-live
///   ratchet still covers engine drift).
///
/// EVERY failure to establish a baseline is an error. This function never
/// returns an "unknown, assume fine" value: a gate that cannot find its
/// baseline and passes anyway is the same bug it exists to prevent.
pub fn resolve_baseline_matrix(root: &Path) -> Result<(String, Tier1Matrix), String> {
    resolve_baseline_matrix_against(root, &baseline_ref())
}

/// [`resolve_baseline_matrix`] with the trunk ref passed explicitly, so callers
/// (and tests) never have to mutate the process environment to choose one.
pub fn resolve_baseline_matrix_against(
    root: &Path,
    base_ref: &str,
) -> Result<(String, Tier1Matrix), String> {
    // A shallow clone cannot be trusted to resolve a merge base or to walk the
    // file's history, and a wrong baseline is indistinguishable from a green
    // gate. Fail loudly with the fix instead of guessing.
    if git(root, &["rev-parse", "--is-shallow-repository"])? == "true" {
        return Err(format!(
            "tier1 ratchet: this is a SHALLOW clone, so the baseline cannot be established. \
             Deepen it first (`git fetch --unshallow origin` or \
             `actions/checkout` with `fetch-depth: 0`) and make sure `{base_ref}` exists. \
             Refusing to run: a gate that cannot find its baseline must not pass."
        ));
    }

    let head = git(root, &["rev-parse", "HEAD"])?;
    let merge_base = git(root, &["merge-base", "HEAD", base_ref]).map_err(|e| {
        format!(
            "tier1 ratchet: cannot compute merge-base(HEAD, {base_ref}): {e}. \
             Fetch the baseline ref (`git fetch --no-tags origin \
             +refs/heads/main:refs/remotes/origin/main`) or point {BASELINE_REF_ENV} \
             at a ref that exists. Refusing to run without a baseline."
        )
    })?;

    let (commit, how) = if merge_base != head {
        (merge_base.clone(), format!("merge-base with {base_ref}"))
    } else {
        // On the trunk: the newest commit that touched the matrix, and the one
        // before it.
        let log = git(
            root,
            &[
                "log",
                "--format=%H",
                "--first-parent",
                "HEAD",
                "--",
                MATRIX_PATH,
            ],
        )?;
        let revs: Vec<&str> = log.lines().collect();
        match revs.first() {
            // HEAD itself changed the matrix — measure against what it replaced.
            Some(&newest) if newest == head => match revs.get(1) {
                Some(&prev) => (
                    prev.to_string(),
                    "previous recorded matrix (HEAD is on the baseline branch)".to_string(),
                ),
                // The commit that introduced the file. There is no earlier
                // recorded state, so there are no `pass` cells to protect.
                None => {
                    return Ok((
                        "no prior matrix revision (file introduced by HEAD)".to_string(),
                        Tier1Matrix::default(),
                    ))
                }
            },
            // The matrix is unchanged at HEAD: nothing new to ratchet. The
            // committed-vs-live ratchet still guards engine drift.
            Some(&newest) => (
                newest.to_string(),
                "newest recorded matrix (unchanged at HEAD)".to_string(),
            ),
            None => {
                return Ok((
                    "no recorded matrix revision".to_string(),
                    Tier1Matrix::default(),
                ))
            }
        }
    };

    let blob = git(root, &["show", &format!("{commit}:{MATRIX_PATH}")]).map_err(|e| {
        format!("tier1 ratchet: cannot read {MATRIX_PATH} at baseline {commit}: {e}")
    })?;
    let matrix: Tier1Matrix = serde_json::from_str(&blob).map_err(|e| {
        format!("tier1 ratchet: baseline {MATRIX_PATH} at {commit} does not parse: {e}")
    })?;
    Ok((
        format!("{} ({how})", &commit[..commit.len().min(12)]),
        matrix,
    ))
}

/// Run the baseline gate: every `pass` the baseline records must still pass
/// live, unless an ack names it. Returns the list of failure lines — empty
/// means the gate is green.
///
/// Failures come in four flavours, all fatal:
/// 1. an unacknowledged drop;
/// 2. an ack whose `to` does not match what the live run reported (an ack for
///    `partial` must not silently cover a slide to `blocked`);
/// 3. an ack with no substantive `reason`, or a malformed `cell`;
/// 4. a STALE ack — one that matches no live regression. Stale acks are an
///    error so the ratchet re-arms the moment a cell recovers, and so drops
///    cannot be pre-acknowledged before they happen.
pub fn baseline_gate_failures(
    baseline: &Tier1Matrix,
    live: &Tier1Matrix,
    acks: &[RatchetAck],
) -> Vec<String> {
    let regressions = baseline_regressions(baseline, live);
    let mut failures = Vec::new();
    let mut matched: BTreeSet<String> = BTreeSet::new();

    for ack in acks {
        if ack.cell.split('/').count() != 2 || ack.cell.split('/').any(|p| p.is_empty()) {
            failures.push(format!(
                "{RATCHET_ACK_PATH}: malformed cell {:?} (want `chip/class`)",
                ack.cell
            ));
            continue;
        }
        if ack.reason.trim().len() < MIN_ACK_REASON_LEN {
            failures.push(format!(
                "{RATCHET_ACK_PATH}: ack for {} has no substantive reason \
                 (need >= {MIN_ACK_REASON_LEN} chars saying why the drop is intentional)",
                ack.cell
            ));
            continue;
        }
        match regressions.iter().find(|r| r.cell() == ack.cell) {
            Some(r) if r.to.as_str() == ack.to => {
                matched.insert(ack.cell.clone());
            }
            Some(r) => failures.push(format!(
                "{RATCHET_ACK_PATH}: ack for {} records to={:?} but the live run reports {:?}. \
                 An ack must cover the drop actually observed.",
                ack.cell,
                ack.to,
                r.to.as_str()
            )),
            None => failures.push(format!(
                "{RATCHET_ACK_PATH}: STALE ack for {} — no such regression in this run. \
                 Delete it (the cell recovered, and the ratchet must re-arm).",
                ack.cell
            )),
        }
    }

    for r in &regressions {
        if !matched.contains(&r.cell()) {
            failures.push(format!("{r}"));
        }
    }
    failures
}

/// One matrix target. Paths are workspace-root-relative.
pub struct Tier1Target {
    pub chip: &'static str,
    pub chip_yaml: &'static str,
    pub elf: &'static str,
    /// Flash image for `--rom-boot` (None = fast-boot ELF entry).
    pub flash_bin: Option<&'static str>,
    pub rom_boot: bool,
    pub max_steps: u64,
    /// Beachhead classes beyond RUBRIC_CLASSES (spec wedge-alignment §4).
    pub extra_classes: &'static [&'static str],
}

/// Shorthand for the common fast-boot, rubric-only target shape.
const fn fast_boot(chip: &'static str, chip_yaml: &'static str, elf: &'static str) -> Tier1Target {
    Tier1Target {
        chip,
        chip_yaml,
        elf,
        flash_bin: None,
        rom_boot: false,
        max_steps: 8_000_000,
        extra_classes: &[],
    }
}

impl Tier1Target {
    /// Beachhead classes on top of a `fast_boot` shape.
    const fn with_extra_classes(mut self, extra: &'static [&'static str]) -> Self {
        self.extra_classes = extra;
        self
    }
}

// One row per SILICON — board variants share their chip's row
// (esp32s3-zero → esp32s3, stm32f401cdu6 → stm32f401).
// Targets whose fixture ELF is not committed yet appear in the matrix as
// full rows of `unrecorded` cells: visible breadth, zero claims.
pub const TIER1_TARGETS: &[Tier1Target] = &[
    Tier1Target {
        chip: "esp32s3",
        chip_yaml: "configs/chips/esp32s3.yaml",
        elf: "tests/fixtures/tier1/esp32s3.elf",
        flash_bin: Some("tests/fixtures/tier1/esp32s3-flash.bin"),
        rom_boot: true,
        // Real ROM + bootloader + app + self-tests. Measured: the full TIER1
        // transcript lands between 16M and 24M steps; 30M = measured + headroom.
        max_steps: 30_000_000,
        extra_classes: &["rmt"],
    },
    fast_boot(
        "esp32",
        "configs/chips/esp32.yaml",
        "tests/fixtures/tier1/esp32.elf",
    ),
    fast_boot(
        "esp32c3",
        "configs/chips/esp32c3.yaml",
        "tests/fixtures/tier1/esp32c3.elf",
    ),
    fast_boot(
        "nrf52832",
        "configs/chips/nrf52832.yaml",
        "tests/fixtures/tier1/nrf52832.elf",
    ),
    fast_boot(
        "nrf52840",
        "configs/chips/nrf52840.yaml",
        "tests/fixtures/tier1/nrf52840.elf",
    ),
    fast_boot(
        "rp2040",
        "configs/chips/rp2040.yaml",
        "tests/fixtures/tier1/rp2040.elf",
    ),
    fast_boot(
        "stm32f103",
        "configs/chips/stm32f103.yaml",
        "tests/fixtures/tier1/stm32f103.elf",
    ),
    fast_boot(
        "stm32f401",
        "configs/chips/stm32f401.yaml",
        "tests/fixtures/tier1/stm32f401.elf",
    ),
    fast_boot(
        "stm32f407",
        "configs/chips/stm32f407.yaml",
        "tests/fixtures/tier1/stm32f407.elf",
    ),
    // WeAct F411 Black Pill. Same silicon row as the F401 plus SPI5 (the `spi`
    // check covers both instances); sim-derived, no bench part.
    fast_boot(
        "stm32f411",
        "configs/chips/stm32f411ceu6.yaml",
        "tests/fixtures/tier1/stm32f411.elf",
    ),
    fast_boot(
        "stm32g474re",
        "configs/chips/stm32g474re.yaml",
        "tests/fixtures/tier1/stm32g474re.elf",
    ),
    fast_boot(
        "stm32h563",
        "configs/chips/stm32h563.yaml",
        "tests/fixtures/tier1/stm32h563.elf",
    )
    .with_extra_classes(&["can"]),
    // First fully-modelled Cortex-M7 chip. H7-family (RM0468); sim-derived
    // (no bench part). Exercises clock/gpio/timer/pwm/i2c/spi/wdt/irq + uart.
    fast_boot(
        "stm32h735",
        "configs/chips/stm32h735.yaml",
        "tests/fixtures/tier1/stm32h735.elf",
    ),
    fast_boot(
        "stm32l073",
        "configs/chips/stm32l073.yaml",
        "tests/fixtures/tier1/stm32l073.elf",
    ),
    fast_boot(
        "stm32l476",
        "configs/chips/stm32l476.yaml",
        "tests/fixtures/tier1/stm32l476.elf",
    ),
    fast_boot(
        "stm32wb55",
        "configs/chips/stm32wb55.yaml",
        "tests/fixtures/tier1/stm32wb55.elf",
    ),
    fast_boot(
        "stm32wba52",
        "configs/chips/stm32wba52.yaml",
        "tests/fixtures/tier1/stm32wba52.elf",
    ),
];

/// Workspace root = two parents up from the cli crate (crates/cli → core).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

#[derive(Deserialize)]
struct ManifestEntry {
    sha256: String,
}

/// Verify every blob listed in `<dir>/MANIFEST.json` against its sha256.
/// Returns the set of verified file names on success, or Err naming the first
/// mismatching file. The returned set is used by `run_all` to check that every
/// blob a target uses is explicitly listed.
pub fn verify_fixture_manifest(dir: &Path) -> Result<BTreeSet<String>, String> {
    let manifest_path = dir.join("MANIFEST.json");
    let manifest: BTreeMap<String, ManifestEntry> = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    for (file, entry) in &manifest {
        let bytes = std::fs::read(dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
        let got = format!("{:x}", Sha256::digest(&bytes));
        if got != entry.sha256 {
            return Err(format!(
                "{file}: sha256 mismatch (manifest {}, got {got})",
                entry.sha256
            ));
        }
    }
    Ok(manifest.into_keys().collect())
}

/// Run one target through the `labwired` binary and parse its TIER1 row.
/// `labwired_bin` lets integration tests pass `env!("CARGO_BIN_EXE_labwired")`.
pub fn run_target(
    target: &Tier1Target,
    labwired_bin: &Path,
) -> Result<BTreeMap<String, Cell>, String> {
    let root = workspace_root();
    let mut cmd = std::process::Command::new(labwired_bin);
    cmd.arg("run")
        .arg("--chip")
        .arg(root.join(target.chip_yaml))
        .arg("--firmware")
        .arg(root.join(target.elf))
        .arg("--max-steps")
        .arg(target.max_steps.to_string());

    // Scrub any inherited LABWIRED_* vars so the matrix is deterministic
    // regardless of the caller's shell environment, then set only the ones
    // this target actually needs.
    for (key, _) in std::env::vars() {
        if key.starts_with("LABWIRED_") {
            cmd.env_remove(&key);
        }
    }

    if target.rom_boot {
        cmd.arg("--rom-boot");
        let flash = target.flash_bin.ok_or("rom_boot target needs flash_bin")?;
        cmd.env("LABWIRED_ESP32S3_FLASH", root.join(flash));
    }
    let out = cmd
        .output()
        .map_err(|e| format!("spawn {}: {e}", labwired_bin.display()))?;

    // UART echoes on stdout; the sim may exit nonzero on step-limit — that's
    // fine, the protocol lines are the verdict.
    //
    // No wall-clock timeout here: step-count bound is sufficient because the
    // sim step loop has no blocking paths (no I/O waits, no sleeps).
    let parsed = parse_tier1_uart(&out.stdout);

    // A crash (non-zero exit, no TIER1 output, no `done`) must surface as an
    // error rather than silently producing a row of Blocked cells.
    if parsed.classes.is_empty() && !parsed.done && !out.status.success() {
        let stderr_tail = {
            let s = String::from_utf8_lossy(&out.stderr);
            let trimmed = s.trim_end();
            if trimmed.len() > 500 {
                let cut = trimmed.len().saturating_sub(500);
                let cut = (cut..=trimmed.len())
                    .find(|&i| trimmed.is_char_boundary(i))
                    .unwrap_or(trimmed.len());
                trimmed[cut..].to_string()
            } else {
                trimmed.to_string()
            }
        };
        return Err(format!(
            "{}: labwired exited {} with no TIER1 output; stderr tail: {}",
            target.chip, out.status, stderr_tail,
        ));
    }

    let classes: Vec<&str> = RUBRIC_CLASSES
        .iter()
        .chain(target.extra_classes.iter())
        .copied()
        .collect();
    let mut row = parsed.resolve_row(&classes);
    let chip_yaml =
        std::fs::read_to_string(root.join(target.chip_yaml)).map_err(|e| e.to_string())?;
    apply_na(&mut row, &declared_classes_from_yaml(&chip_yaml)?);
    Ok(row)
}

/// Run every target whose fixture blobs exist. Returns the live matrix and the
/// list of skipped chips (missing fixtures — fresh clone or fixtures not landed yet).
pub fn run_all(labwired_bin: &Path) -> Result<(Tier1Matrix, Vec<String>), String> {
    let root = workspace_root();
    let fixture_dir = root.join("tests/fixtures/tier1");

    // Determine which targets have ELF files present.
    let any_elf_present = TIER1_TARGETS.iter().any(|t| root.join(t.elf).exists());

    // If any ELF exists, MANIFEST.json is mandatory and must cover every blob
    // that a non-skipped target uses.
    let verified: Option<BTreeSet<String>> = if any_elf_present {
        let manifest_path = fixture_dir.join("MANIFEST.json");
        if !manifest_path.exists() {
            return Err(format!(
                "MANIFEST.json is required when fixture ELFs are present but was not found at {}",
                manifest_path.display()
            ));
        }
        Some(verify_fixture_manifest(&fixture_dir)?)
    } else {
        None
    };

    // Before running any target, verify that every blob it uses is listed in
    // the manifest — an omitted blob is an error naming the file.
    if let Some(ref listed) = verified {
        for target in TIER1_TARGETS {
            if !root.join(target.elf).exists() {
                continue; // will be skipped below
            }
            let elf_name = Path::new(target.elf)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(target.elf);
            if !listed.contains(elf_name) {
                return Err(format!(
                    "fixture blob '{elf_name}' used by target '{}' is not listed in MANIFEST.json",
                    target.chip
                ));
            }
            if let Some(flash) = target.flash_bin {
                let flash_name = Path::new(flash)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(flash);
                if !listed.contains(flash_name) {
                    return Err(format!(
                        "fixture blob '{flash_name}' used by target '{}' is not listed in MANIFEST.json",
                        target.chip
                    ));
                }
            }
        }
    }

    let mut matrix = Tier1Matrix::default();
    let mut skipped = Vec::new();
    for target in TIER1_TARGETS {
        if !root.join(target.elf).exists() {
            // Planned-but-unfixtured silicon stays VISIBLE: a full row of
            // `unrecorded` cells (breadth without claims) instead of being
            // silently absent. The ratchet ignores unrecorded; the scoreboard
            // and /validation page render `·`.
            skipped.push(target.chip.to_string());
            matrix
                .0
                .insert(target.chip.to_string(), unrecorded_row(target));
            continue;
        }
        let row = run_target(target, labwired_bin)?;
        matrix.0.insert(target.chip.to_string(), row);
    }
    Ok((matrix, skipped))
}

/// Full row of `unrecorded` cells for a target with no committed fixture.
fn unrecorded_row(target: &Tier1Target) -> BTreeMap<String, Cell> {
    RUBRIC_CLASSES
        .iter()
        .chain(target.extra_classes.iter())
        .map(|class| {
            (
                class.to_string(),
                Cell {
                    status: CellStatus::Unrecorded,
                    run_url: None,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_targets_emit_full_unrecorded_rows() {
        let target = &TIER1_TARGETS[1]; // a planned fast-boot target
        let row = unrecorded_row(target);
        assert_eq!(row.len(), RUBRIC_CLASSES.len() + target.extra_classes.len());
        assert!(row
            .values()
            .all(|c| c.status == CellStatus::Unrecorded && c.run_url.is_none()));
    }

    #[test]
    fn parses_pass_fail_lines_and_done() {
        let uart =
            b"boot noise\nTIER1 clock PASS\nTIER1 gpio PASS\nTIER1 dma FAIL code=gdma-idle\nTIER1 done\ntrailing";
        let parsed = parse_tier1_uart(uart);
        assert!(parsed.done);
        assert_eq!(parsed.classes["clock"], CellStatus::Pass);
        assert_eq!(parsed.classes["gpio"], CellStatus::Pass);
        assert_eq!(parsed.classes["dma"], CellStatus::Blocked);
    }

    #[test]
    fn missing_done_marks_row_partial_for_reported_passes() {
        let uart = b"TIER1 clock PASS\nTIER1 gpio PASS\n"; // hung before done
        let parsed = parse_tier1_uart(uart);
        assert!(!parsed.done);
        let row = parsed.resolve_row(&["clock", "gpio", "uart"]);
        // reported passes degrade to partial; unreported classes are blocked
        assert_eq!(row["clock"].status, CellStatus::Partial);
        assert_eq!(row["gpio"].status, CellStatus::Partial);
        assert_eq!(row["uart"].status, CellStatus::Blocked);
    }

    #[test]
    fn no_tier1_lines_blocks_uart_and_everything_else() {
        let parsed = parse_tier1_uart(b"garbage \xff\xfe binary noise");
        assert!(!parsed.done);
        assert!(parsed.classes.is_empty());
        let row = parsed.resolve_row(RUBRIC_CLASSES);
        for class in RUBRIC_CLASSES {
            assert_eq!(row[*class].status, CellStatus::Blocked, "{class}");
        }
    }

    #[test]
    fn garbage_tier1_lines_are_ignored_not_fatal() {
        let uart = b"TIER1 gpio MAYBE\nTIER1\nTIER1 gpio PASS\nTIER1 done\n";
        let parsed = parse_tier1_uart(uart);
        assert_eq!(parsed.classes["gpio"], CellStatus::Pass);
        assert_eq!(parsed.classes.len(), 1);
    }

    #[test]
    fn uart_class_is_implicitly_pass_when_done_arrives() {
        // The fixture never prints "TIER1 uart ..." — receiving the protocol IS the proof.
        let parsed = parse_tier1_uart(b"TIER1 clock PASS\nTIER1 done\n");
        let row = parsed.resolve_row(&["clock", "uart"]);
        assert_eq!(row["uart"].status, CellStatus::Pass);
    }

    #[test]
    fn explicit_uart_fail_wins_over_implicit_rule() {
        let parsed =
            parse_tier1_uart(b"TIER1 clock PASS\nTIER1 uart FAIL code=parity\nTIER1 done\n");
        let row = parsed.resolve_row(&["clock", "uart"]);
        assert_eq!(row["uart"].status, CellStatus::Blocked);
    }

    #[test]
    fn done_alone_proves_uart() {
        let parsed = parse_tier1_uart(b"TIER1 done\n");
        let row = parsed.resolve_row(&["uart", "gpio"]);
        assert_eq!(row["uart"].status, CellStatus::Pass);
        // Unattempted class with done seen: no claim (NOT blocked) — blocked
        // is reserved for explicit FAILs and hung-before-done sequences.
        assert_eq!(row["gpio"].status, CellStatus::Unrecorded);
    }

    #[test]
    fn duplicate_class_lines_last_wins() {
        let parsed = parse_tier1_uart(b"TIER1 gpio PASS\nTIER1 gpio FAIL code=retry\nTIER1 done\n");
        assert_eq!(parsed.classes["gpio"], CellStatus::Blocked);
        // and the reverse: a retry that recovers
        let parsed = parse_tier1_uart(b"TIER1 gpio FAIL code=first\nTIER1 gpio PASS\nTIER1 done\n");
        assert_eq!(parsed.classes["gpio"], CellStatus::Pass);
    }

    #[test]
    fn whitespace_and_crlf_are_tolerated() {
        let parsed = parse_tier1_uart(b"  TIER1\tclock   PASS\r\nTIER1 done\r\n");
        assert_eq!(parsed.classes["clock"], CellStatus::Pass);
        assert!(parsed.done);
    }

    #[test]
    fn derives_na_from_chip_yaml_peripheral_ids() {
        // Minimal chip yaml shape — only `peripherals[].id` matters here.
        let yaml = r#"
name: "fakechip"
arch: "xtensa"
peripherals:
  - { id: "uart0", type: "uart", base_address: 0x60000000 }
  - { id: "gpio", type: "declarative", base_address: 0x60004000 }
  - { id: "timg0", type: "declarative", base_address: 0x6001F000 }
  - { id: "interrupt_core0", type: "declarative", base_address: 0x600C2000 }
"#;
        let declared = declared_classes_from_yaml(yaml).unwrap();
        assert!(declared.contains("uart"));
        assert!(declared.contains("gpio"));
        assert!(declared.contains("timer"));
        assert!(declared.contains("irq"));
        assert!(!declared.contains("dma")); // not declared → n/a, not blocked
        assert!(!declared.contains("mcpwm"));
    }

    #[test]
    fn na_overrides_blocked_in_row_resolution() {
        let parsed = parse_tier1_uart(b"TIER1 clock PASS\nTIER1 done\n");
        let mut row = parsed.resolve_row(RUBRIC_CLASSES);
        let declared: BTreeSet<String> = ["clock", "uart"].iter().map(|s| s.to_string()).collect();
        apply_na(&mut row, &declared);
        assert_eq!(row["clock"].status, CellStatus::Pass);
        assert_eq!(row["dma"].status, CellStatus::Na); // undeclared
        assert_eq!(row["gpio"].status, CellStatus::Na); // undeclared
    }

    fn cell(s: CellStatus) -> Cell {
        Cell {
            status: s,
            run_url: None,
        }
    }

    #[test]
    fn ratchet_flags_pass_regressions_only() {
        let mut snap = Tier1Matrix::default();
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Pass));
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("dma".into(), cell(CellStatus::Blocked));

        let mut live = Tier1Matrix::default();
        live.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Blocked)); // regression!
        live.0
            .entry("esp32s3".into())
            .or_default()
            .insert("dma".into(), cell(CellStatus::Pass)); // improvement — fine

        let regressions = ratchet_regressions(&snap, &live);
        assert_eq!(
            regressions,
            vec!["esp32s3/gpio: pass -> blocked".to_string()]
        );
    }

    #[test]
    fn ratchet_ignores_unrecorded_and_missing_chips() {
        let mut snap = Tier1Matrix::default();
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Unrecorded));
        let live = Tier1Matrix::default(); // chip absent from live run
        assert!(ratchet_regressions(&snap, &live).is_empty());
    }

    #[test]
    fn snapshot_roundtrip_is_deterministic() {
        let mut m = Tier1Matrix::default();
        m.0.entry("b".into())
            .or_default()
            .insert("z".into(), cell(CellStatus::Pass));
        m.0.entry("a".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Na));
        let j1 = serde_json::to_string_pretty(&m).unwrap();
        let j2 = serde_json::to_string_pretty(&serde_json::from_str::<Tier1Matrix>(&j1).unwrap())
            .unwrap();
        assert_eq!(j1, j2);
        assert!(j1.find("\"a\"").unwrap() < j1.find("\"b\"").unwrap());
    }

    #[test]
    fn skipped_chips_with_passes_detects_disarmed_fixture() {
        // Snapshot has esp32s3 with a pass cell — it gets flagged when skipped.
        let mut snap = Tier1Matrix::default();
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Pass));

        let skipped = vec!["esp32s3".to_string(), "other".to_string()];
        let disarmed = skipped_chips_with_recorded_passes(&snap, &skipped);
        assert_eq!(disarmed, vec!["esp32s3".to_string()]);
    }

    #[test]
    fn skipped_chips_with_only_blocked_cells_not_flagged() {
        // Snapshot has a chip but only blocked/na cells — not a disarmed gate.
        let mut snap = Tier1Matrix::default();
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("gpio".into(), cell(CellStatus::Blocked));
        snap.0
            .entry("esp32s3".into())
            .or_default()
            .insert("dma".into(), cell(CellStatus::Na));

        let skipped = vec!["esp32s3".to_string()];
        let disarmed = skipped_chips_with_recorded_passes(&snap, &skipped);
        assert!(disarmed.is_empty());
    }

    // ── merge-base baseline ratchet ──────────────────────────────────────

    fn matrix(rows: &[(&str, &[(&str, CellStatus)])]) -> Tier1Matrix {
        let mut m = Tier1Matrix::default();
        for (chip, cells) in rows {
            let row = m.0.entry((*chip).to_string()).or_default();
            for (class, status) in *cells {
                row.insert((*class).to_string(), cell(*status));
            }
        }
        m
    }

    fn ack(cell: &str, to: &str) -> RatchetAck {
        RatchetAck {
            cell: cell.to_string(),
            to: to.to_string(),
            reason: "fixture retired on purpose; tracked in the follow-up issue".into(),
        }
    }

    /// THE BUG THIS GATE EXISTS FOR: regenerating the snapshot makes the
    /// committed-vs-live ratchet vacuous, while the baseline gate still bites.
    #[test]
    fn regenerating_the_snapshot_defeats_the_old_ratchet_but_not_the_baseline() {
        let baseline = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]);
        // The change under test broke i2c AND regenerated the snapshot, so the
        // committed snapshot and the live run agree.
        let live = matrix(&[("esp32c3", &[("i2c", CellStatus::Blocked)])]);
        let regenerated_snapshot = live.clone();

        assert!(
            ratchet_regressions(&regenerated_snapshot, &live).is_empty(),
            "old ratchet is vacuous once the snapshot is regenerated"
        );
        assert_eq!(
            baseline_gate_failures(&baseline, &live, &[]),
            vec!["esp32c3/i2c: pass -> blocked".to_string()]
        );
    }

    #[test]
    fn baseline_gate_is_green_when_nothing_dropped() {
        let baseline = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]);
        let live = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]);
        assert!(baseline_gate_failures(&baseline, &live, &[]).is_empty());
    }

    #[test]
    fn improvements_and_non_pass_baseline_cells_move_freely() {
        let baseline = matrix(&[(
            "esp32c3",
            &[("i2c", CellStatus::Blocked), ("spi", CellStatus::Partial)],
        )]);
        let live = matrix(&[(
            "esp32c3",
            &[("i2c", CellStatus::Pass), ("spi", CellStatus::Blocked)],
        )]);
        assert!(baseline_gate_failures(&baseline, &live, &[]).is_empty());
    }

    #[test]
    fn every_non_pass_status_counts_as_a_drop() {
        let baseline = matrix(&[(
            "c",
            &[
                ("a", CellStatus::Pass),
                ("b", CellStatus::Pass),
                ("d", CellStatus::Pass),
                ("e", CellStatus::Pass),
            ],
        )]);
        let live = matrix(&[(
            "c",
            &[
                ("a", CellStatus::Partial),
                ("b", CellStatus::Blocked),
                ("d", CellStatus::Unrecorded),
                ("e", CellStatus::Na),
            ],
        )]);
        assert_eq!(
            baseline_gate_failures(&baseline, &live, &[]),
            vec![
                "c/a: pass -> partial",
                "c/b: pass -> blocked",
                "c/d: pass -> unrecorded",
                "c/e: pass -> na",
            ]
        );
    }

    #[test]
    fn a_named_ack_covers_its_drop() {
        let baseline = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]);
        let live = matrix(&[("esp32c3", &[("i2c", CellStatus::Blocked)])]);
        assert!(
            baseline_gate_failures(&baseline, &live, &[ack("esp32c3/i2c", "blocked")]).is_empty()
        );
    }

    #[test]
    fn an_ack_does_not_cover_a_different_drop() {
        let baseline = matrix(&[(
            "esp32c3",
            &[("i2c", CellStatus::Pass), ("pwm", CellStatus::Pass)],
        )]);
        let live = matrix(&[(
            "esp32c3",
            &[("i2c", CellStatus::Blocked), ("pwm", CellStatus::Blocked)],
        )]);
        // Silence on pwm is not sufficient just because i2c was acked.
        assert_eq!(
            baseline_gate_failures(&baseline, &live, &[ack("esp32c3/i2c", "blocked")]),
            vec!["esp32c3/pwm: pass -> blocked".to_string()]
        );
    }

    #[test]
    fn an_ack_must_cover_the_status_actually_observed() {
        let baseline = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]);
        let live = matrix(&[("esp32c3", &[("i2c", CellStatus::Blocked)])]);
        // Acked as `partial`, but the cell actually slid all the way to blocked.
        let failures = baseline_gate_failures(&baseline, &live, &[ack("esp32c3/i2c", "partial")]);
        // The mis-scoped ack is rejected AND the drop stays unacknowledged.
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(
            failures[0].contains("records to=\"partial\""),
            "{failures:?}"
        );
        assert!(
            failures[0].contains("live run reports \"blocked\""),
            "{failures:?}"
        );
        assert_eq!(failures[1], "esp32c3/i2c: pass -> blocked");
    }

    #[test]
    fn a_stale_ack_is_an_error_so_the_ratchet_re_arms() {
        let baseline = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]);
        let live = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]); // recovered
        let failures = baseline_gate_failures(&baseline, &live, &[ack("esp32c3/i2c", "blocked")]);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].contains("STALE ack"), "{failures:?}");
    }

    #[test]
    fn an_ack_without_a_substantive_reason_is_rejected() {
        let baseline = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]);
        let live = matrix(&[("esp32c3", &[("i2c", CellStatus::Blocked)])]);
        let mut a = ack("esp32c3/i2c", "blocked");
        a.reason = "wip".into();
        let failures = baseline_gate_failures(&baseline, &live, &[a]);
        // Rejected as an ack AND the drop still reported.
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(
            failures[0].contains("no substantive reason"),
            "{failures:?}"
        );
        assert_eq!(failures[1], "esp32c3/i2c: pass -> blocked");
    }

    #[test]
    fn a_malformed_ack_cell_is_rejected() {
        let baseline = matrix(&[("esp32c3", &[("i2c", CellStatus::Pass)])]);
        let live = matrix(&[("esp32c3", &[("i2c", CellStatus::Blocked)])]);
        let mut a = ack("esp32c3-i2c", "blocked");
        a.cell = "esp32c3-i2c".into();
        let failures = baseline_gate_failures(&baseline, &live, &[a]);
        assert!(failures[0].contains("malformed cell"), "{failures:?}");
    }

    #[test]
    fn ack_file_absent_means_no_acks_but_malformed_is_fatal() {
        let dir = std::env::temp_dir().join(format!("tier1-acks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/coverage")).unwrap();
        assert!(load_ratchet_acks(&dir).unwrap().is_empty(), "absent = none");

        std::fs::write(
            dir.join(RATCHET_ACK_PATH),
            "acks: [ this is not: a list ]\n",
        )
        .unwrap();
        assert!(
            load_ratchet_acks(&dir).is_err(),
            "a broken ack file must never read as an empty one"
        );

        std::fs::write(
            dir.join(RATCHET_ACK_PATH),
            "acks:\n  - cell: esp32c3/i2c\n    to: blocked\n    reason: \"because reasons that are long enough\"\n",
        )
        .unwrap();
        let acks = load_ratchet_acks(&dir).unwrap();
        assert_eq!(acks.len(), 1);
        assert_eq!(acks[0].cell, "esp32c3/i2c");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE SHALLOW-CLONE TRAP. CI checkouts are shallow by default, and on a
    /// shallow clone a merge base is either unresolvable or wrong. Either way
    /// the gate must refuse to run rather than report "no regressions".
    #[test]
    fn baseline_refuses_to_run_on_a_shallow_clone() {
        let dir = std::env::temp_dir().join(format!("tier1-shallow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
        ] {
            git(&dir, &args).unwrap();
        }
        std::fs::write(dir.join("f"), "x").unwrap();
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &["commit", "-qm", "seed", "--no-verify"]).unwrap();
        // What `git clone --depth` leaves behind; `rev-parse
        // --is-shallow-repository` keys on exactly this file.
        std::fs::write(dir.join(".git/shallow"), "").unwrap();

        let err = resolve_baseline_matrix_against(&dir, DEFAULT_BASELINE_REF)
            .expect_err("a shallow clone must be an error, never a silent pass");
        assert!(err.contains("SHALLOW"), "{err}");
        assert!(err.contains("fetch-depth: 0"), "{err}");
        assert!(err.contains("must not pass"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn baseline_resolves_in_this_repo_and_parses() {
        // Runs against the real checkout: proves the git plumbing works and
        // that whatever it returns is a parseable matrix.
        let (desc, baseline) =
            resolve_baseline_matrix_against(&workspace_root(), DEFAULT_BASELINE_REF)
                .unwrap_or_else(|e| panic!("baseline resolution failed: {e}"));
        assert!(!desc.is_empty());
        // Either a real matrix, or the documented "no prior revision" empty one.
        for row in baseline.0.values() {
            assert!(!row.is_empty());
        }
    }

    #[test]
    fn baseline_fails_loudly_on_an_unresolvable_ref() {
        let err = resolve_baseline_matrix_against(
            &workspace_root(),
            "refs/heads/definitely-not-a-real-ref",
        )
        .expect_err("an unresolvable baseline ref must be an error, not a pass");
        assert!(err.contains("merge-base"), "{err}");
        assert!(err.contains("Refusing to run"), "{err}");
    }

    #[test]
    fn baseline_fails_loudly_outside_a_git_repo() {
        let dir = std::env::temp_dir().join(format!("tier1-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = resolve_baseline_matrix_against(&dir, DEFAULT_BASELINE_REF)
            .expect_err("no git repo must be an error, not an empty baseline");
        assert!(err.contains("git"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shipped ack file must always be valid, whatever it contains.
    #[test]
    fn committed_ack_file_is_well_formed() {
        let acks = load_ratchet_acks(&workspace_root()).expect("ack file parses");
        for a in &acks {
            assert_eq!(a.cell.split('/').count(), 2, "{:?}", a.cell);
            assert!(
                a.reason.trim().len() >= MIN_ACK_REASON_LEN,
                "ack for {} needs a substantive reason",
                a.cell
            );
            assert!(
                ["pass", "partial", "blocked", "na", "unrecorded"].contains(&a.to.as_str()),
                "ack for {} has unknown status {:?}",
                a.cell,
                a.to
            );
        }
    }

    #[test]
    fn cell_status_as_str_matches_serde() {
        assert_eq!(serde_json::to_string(&CellStatus::Na).unwrap(), "\"na\"");
        assert_eq!(CellStatus::Na.as_str(), "na");
    }

    #[test]
    fn target_table_paths_resolve_relative_to_workspace_root() {
        let root = workspace_root();
        for t in TIER1_TARGETS {
            assert!(
                t.chip_yaml.ends_with(".yaml"),
                "{}: chip_yaml does not end with .yaml",
                t.chip
            );
            assert!(
                root.join(t.chip_yaml).exists(),
                "{}: chip_yaml {} does not exist",
                t.chip,
                t.chip_yaml
            );
        }
        let t = &TIER1_TARGETS[0];
        assert_eq!(t.chip, "esp32s3");
        assert!(t.chip_yaml.ends_with("configs/chips/esp32s3.yaml"));
        assert!(t.elf.ends_with("tests/fixtures/tier1/esp32s3.elf"));
        assert!(t
            .flash_bin
            .unwrap()
            .ends_with("tests/fixtures/tier1/esp32s3-flash.bin"));
    }

    /// Cross-registry consistency (see also
    /// crates/core/tests/board_registry_consistency.rs, which cannot import
    /// TIER1_TARGETS across an integration-test binary boundary — and taking
    /// labwired-core as a dev-dependency of labwired-cli would be a cycle, so
    /// this half of the check lives here instead).
    #[test]
    fn every_tier1_target_is_a_known_manifest_board() {
        let text = std::fs::read_to_string(workspace_root().join("validation/manifest.yaml"))
            .expect("read validation/manifest.yaml");
        let mut manifest = BTreeSet::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("chip:") {
                let stem = Path::new(rest.trim())
                    .file_stem()
                    .unwrap_or_else(|| panic!("malformed chip path in manifest: {rest}"))
                    .to_string_lossy()
                    .to_string();
                manifest.insert(stem);
            }
        }
        assert!(
            !manifest.is_empty(),
            "parsed no chip: entries from the manifest"
        );

        for target in TIER1_TARGETS {
            let stem = Path::new(target.chip_yaml)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();
            assert!(
                manifest.contains(&stem),
                "TIER1_TARGETS builds the matrix for chip {stem:?} but no board in \
                 validation/manifest.yaml declares it"
            );
        }
    }

    #[test]
    fn manifest_verification_rejects_corrupt_blob() {
        let dir =
            std::env::temp_dir().join(format!("tier1-manifest-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("esp32s3.elf"), b"not the real elf").unwrap();
        let manifest = r#"{ "esp32s3.elf": { "sha256": "0000000000000000000000000000000000000000000000000000000000000000" } }"#;
        std::fs::write(dir.join("MANIFEST.json"), manifest).unwrap();
        let err = verify_fixture_manifest(&dir).unwrap_err();
        assert!(err.contains("esp32s3.elf"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_verification_happy_path() {
        let dir = std::env::temp_dir().join(format!("tier1-manifest-happy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body = b"good-blob-bytes";
        std::fs::write(dir.join("esp32s3.elf"), body).unwrap();
        let sha = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(body));
        std::fs::write(
            dir.join("MANIFEST.json"),
            format!(r#"{{ "esp32s3.elf": {{ "sha256": "{sha}" }} }}"#),
        )
        .unwrap();
        let verified = verify_fixture_manifest(&dir).unwrap();
        assert!(verified.contains("esp32s3.elf"), "{verified:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_verification_missing_blob_file() {
        let dir =
            std::env::temp_dir().join(format!("tier1-manifest-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // MANIFEST.json references a file that doesn't exist in the dir.
        let manifest = r#"{ "nonexistent.bin": { "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789" } }"#;
        std::fs::write(dir.join("MANIFEST.json"), manifest).unwrap();
        let err = verify_fixture_manifest(&dir).unwrap_err();
        assert!(err.contains("nonexistent.bin"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_all_style_manifest_listing_is_enforced() {
        // verify_fixture_manifest returns the set of verified file names
        let dir = std::env::temp_dir().join(format!("tier1-manifest-list-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body = b"blob-bytes";
        std::fs::write(dir.join("esp32s3.elf"), body).unwrap();
        let sha = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(body));
        std::fs::write(
            dir.join("MANIFEST.json"),
            format!(r#"{{ "esp32s3.elf": {{ "sha256": "{sha}" }} }}"#),
        )
        .unwrap();
        let verified = verify_fixture_manifest(&dir).unwrap();
        assert!(verified.contains("esp32s3.elf"));
        assert!(!verified.contains("esp32s3-flash.bin"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Resolve a committed, already-executable fake `labwired` binary.
    ///
    /// THE ETXTBSY TRAP. These tests used to write the script themselves,
    /// `chmod 0755` it, and then exec it. In a multithreaded test binary that
    /// is a race: `exec` fails with `ETXTBSY` ("Text file busy") if *any*
    /// process still holds the file open for writing. While this thread's
    /// write fd is open, a concurrent `Command::spawn` on another test thread
    /// forks, the child inherits the descriptor, and until that child reaches
    /// its own `exec` (which is where `O_CLOEXEC` finally closes the
    /// inherited fd) our exec is refused. The `git`-backed baseline tests
    /// below fork repeatedly, so the window gets hit under CI parallelism.
    ///
    /// Exec'ing a fixture this process never opens for writing removes the
    /// window by construction — no lock and no retry needed, and no
    /// production code has to change to accommodate a test.
    fn fake_labwired_bin(name: &str) -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        assert!(p.is_file(), "missing committed fixture: {}", p.display());
        // The exec bit is carried by git (mode 100755). If a checkout ever
        // drops it, fail here with the reason rather than as a baffling
        // "Permission denied" inside the assertions below.
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "fixture must stay executable (mode {mode:o}): {}",
            p.display()
        );
        p
    }

    #[test]
    fn run_target_surfaces_child_crash_instead_of_blocked_row() {
        let fake = fake_labwired_bin("tier1-fake-crash.sh");
        let target = &TIER1_TARGETS[0];
        // chip yaml exists in-repo, ELF path doesn't need to exist for the spawn itself
        let err = run_target(target, &fake).unwrap_err();
        assert!(err.contains("boom-stderr"), "{err}");
        assert!(err.contains("esp32s3"), "{err}");
    }

    #[test]
    fn stderr_tail_truncation_is_char_boundary_safe() {
        // The fixture writes >500 bytes of multibyte stderr (U+2744 = 3 bytes
        // each × 200 = 600 bytes) so the naive len-500 cut lands mid-char.
        let fake = fake_labwired_bin("tier1-fake-crash-utf8.sh");
        let target = &TIER1_TARGETS[0];
        let err = run_target(target, &fake).unwrap_err();
        assert!(err.contains("exited"), "{err}");
    }
}
