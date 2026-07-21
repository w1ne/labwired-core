// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! BOARD-COVERAGE RATCHET — "rock solid per board", enforced.
//!
//! Why this file exists
//! ====================
//! A chip can appear in the shipped catalog (`bundled-configs.ts` +
//! `configs/chips/`) with a green CI and still have NO test that actually runs
//! its firmware against a reference. That is exactly how RP2040 and STM32F401
//! shipped: they boot in the playground but nothing pins their core timer/IRQ
//! path to a differential or a silicon oracle. This gate makes that state
//! *visible and un-expandable*: every shipped chip must carry a minimum set of
//! coverage classes, discovered BY CONVENTION (we grep the test corpus for
//! naming patterns), never a hand-maintained manifest that drifts.
//!
//! The required coverage classes (policy)
//! ======================================
//! For every SHIPPED chip:
//!
//! (a) RESET / BOOT CONFORMANCE — a `*conformance*` / `*clock_boot*` / `*reset*`
//! test that references the chip (the fleet-wide `chip_conformance.rs`
//! scoreboard satisfies this for every chip).
//!
//! (b) EXECUTING FIDELITY — at least one test that *runs code* and compares it to
//! a reference on the chip's core timer/IRQ path: a walk-vs-scheduler
//! differential, a silicon exec-oracle / MMIO-diff, a golden trace, or a JIT
//! lockstep. A "firmware survival" boot-and-don't-crash test does NOT count —
//! surviving is not the same as being *right*. A chip that ships with no
//! executing-fidelity test is the RP2040/F401 exposure.
//!
//! (c) DISPLAY ASSERTION — for chips that drive a shipped display lab, a
//! display-level test (`*oled*` / `*nokia*` / `*epaper*` / `*ssd1306*` /
//! framebuffer assert) that references the chip.
//!
//! Naming CONVENTIONS the gap-filler PRs follow (this IS the contract)
//! ==================================================================
//! - walk differentials: `crates/core/tests/<chip>_walk_differential.rs`
//! - silicon oracle/mmio: `crates/hw-oracle/tests/<chip>_exec_oracle.rs`,
//!   `crates/hw-oracle/tests/<chip>_mmio_diff.rs`
//! - reset conformance: the `chip_conformance` scoreboard, or
//!   `crates/hw-oracle/tests/<chip>_reset_conformance.rs`
//! - display differentials: `*_<display>_differential` / framebuffer asserts
//!
//! The KNOWN-GAP allowlist
//! =======================
//! `KNOWN_GAPS` is a *shrink-only* ratchet allowlist, exactly like
//! `strict_onboarding.rs`'s `SMOKE_LESS_ALLOWLIST`. A chip/class listed there is
//! allowed to be missing TODAY — but a NEW uncovered chip/class that is NOT on
//! the list fails the gate, and a listed gap that has since been FILLED also
//! fails the gate (stale entry), so the allowlist can only get shorter. Gap-filler PRs land the missing test,
//! then delete the matching `KNOWN_GAPS` row. Do NOT add rows to make a red gate
//! green — that re-opens the exact hole this file exists to close.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A required coverage class.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Class {
    Reset,
    Exec,
    Display,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Class::Reset => "reset/boot conformance",
            Class::Exec => {
                "executing fidelity (walk-diff / exec-oracle / mmio-diff / golden / lockstep)"
            }
            Class::Display => "display-level assertion",
        }
    }
    /// Filename substrings that mark a test file as belonging to this class.
    fn filename_markers(self) -> &'static [&'static str] {
        match self {
            Class::Reset => &["conformance", "clock_boot", "reset"],
            Class::Exec => &[
                "walk_differential",
                "walk_free_differential",
                "exec_oracle",
                "mmio_diff",
                "lockstep",
                "golden",
                "full_state_differential",
            ],
            Class::Display => &[
                "oled",
                "nokia",
                "epaper",
                "ssd1306",
                "framebuffer",
                "_lcd",
                "display",
            ],
        }
    }
}

/// One shipped chip and the coverage it must carry.
struct Board {
    /// Canonical catalog id.
    chip: &'static str,
    /// Case-insensitive substrings that identify this chip inside a test file
    /// (filename or body). Aliases exist because hw-oracle files are named by
    /// family (`stm32f1_*`, `stm32f4_*`, `stm32l0_*`) rather than by part.
    aliases: &'static [&'static str],
    /// `configs/chips/<stem>.yaml` file stem. A test that LOADS this config is
    /// unambiguously targeting this chip — the strong signal used for the
    /// reset/exec classes, where family-shared files (`stm32f4_*` naming both
    /// F407 and F401 in prose) make a loose prose match unsafe.
    yaml_stem: &'static str,
    /// Does this chip drive a SHIPPED display lab (per `bundled-configs.ts`)?
    /// Only then is the display-assertion class required.
    display_lab: bool,
}

/// The canonical SHIPPED board/chip list (verified against
/// `packages/playground/src/bundled-configs.ts` + `configs/chips/`).
///
/// A chip added to the shipped catalog MUST be added here, or the cross-check at
/// the bottom of this file fails — a shipped board can never bypass the ratchet.
const SHIPPED: &[Board] = &[
    Board {
        chip: "stm32f103",
        aliases: &["stm32f103", "f103", "stm32f1"],
        yaml_stem: "stm32f103",
        display_lab: true,
    },
    Board {
        chip: "stm32f401",
        aliases: &["stm32f401", "f401"],
        yaml_stem: "stm32f401",
        display_lab: false,
    },
    Board {
        chip: "stm32f407",
        aliases: &["stm32f407", "f407", "stm32f4", "nucleo-f407"],
        yaml_stem: "stm32f407",
        display_lab: false,
    },
    Board {
        chip: "stm32l073",
        aliases: &["stm32l073", "l073", "stm32l0"],
        yaml_stem: "stm32l073",
        display_lab: false,
    },
    Board {
        chip: "stm32l476",
        aliases: &["stm32l476", "l476", "stm32l4"],
        yaml_stem: "stm32l476",
        display_lab: true,
    },
    Board {
        chip: "stm32h563",
        aliases: &["stm32h563", "h563"],
        yaml_stem: "stm32h563",
        display_lab: false,
    },
    Board {
        chip: "nrf52840",
        aliases: &["nrf52840", "nrf52_", "nrf52 "],
        yaml_stem: "nrf52840",
        display_lab: false,
    },
    Board {
        chip: "esp32c3",
        aliases: &["esp32c3"],
        yaml_stem: "esp32c3",
        display_lab: true,
    },
    Board {
        chip: "esp32s3",
        aliases: &["esp32s3"],
        yaml_stem: "esp32s3",
        display_lab: true,
    },
    Board {
        chip: "kw41z",
        aliases: &["kw41z", "mkw41z"],
        yaml_stem: "mkw41z4",
        display_lab: false,
    },
    Board {
        chip: "rp2040",
        aliases: &["rp2040"],
        yaml_stem: "rp2040",
        display_lab: false,
    },
];

/// SHRINK-ONLY allowlist of (chip, class) pairs known to lack coverage today.
///
/// Every entry is a debt with a tracking note. Fill the gap (land the named
/// convention test), then DELETE the row — the gate fails if a listed gap is
/// found already covered, so stale rows cannot accumulate.
const KNOWN_GAPS: &[(&str, Class)] = &[
    // EMPTY — every shipped chip now carries its required coverage classes.
    // RP2040 covered by rp2040_{timer_exec_oracle,reset_conformance,pio_onboarding};
    // STM32F401 covered by stm32f401_walk_differential.
    //
    // Two MODELING gaps (not test gaps) are tracked as issues, NOT allowlisted here,
    // because the ratchet must not demand a test for a peripheral the chip doesn't
    // model: RP2040 DMA (issue #577) and STM32F407 DMA (issue #578) have no block in
    // their chip descriptors. F401 cold-reset value divergences vs RM0368: issue #576.
];

fn core_crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `crates/core/tests` and `crates/hw-oracle/tests`,
/// with its filename (lowercased) and full body (lowercased) for scanning.
struct Corpus {
    files: Vec<(String, String)>, // (lowercased filename, lowercased contents)
}

impl Corpus {
    fn load() -> Self {
        let core_tests = core_crate_root().join("tests");
        let oracle_tests = core_crate_root().join("../hw-oracle/tests");
        let mut files = Vec::new();
        for dir in [core_tests, oracle_tests] {
            collect_rs(&dir, &mut files);
        }
        assert!(
            !files.is_empty(),
            "board_coverage_ratchet found no test files — check the corpus paths"
        );
        Corpus { files }
    }

    /// Does a file whose NAME marks it as `class` actually TARGET this chip?
    ///
    /// Matching strength depends on the class:
    ///   * `Reset`/`Exec` — the STRONG signal: the chip appears in the FILENAME
    ///     (a family alias like `stm32f4_*`) OR the file explicitly LOADS this
    ///     chip's config (`chips/<stem>.yaml`). Prose alone does not count,
    ///     because family-shared files name sibling parts (e.g. the F407
    ///     mmio-diff mentions F401 in a comment) — a prose match there would
    ///     falsely credit F401 with an executing-fidelity test it does not have.
    ///   * `Display` — filename alias OR any body reference (prose included):
    ///     display labs name their target board in a header comment
    ///     (`NUCLEO-F103RB`, `NUCLEO-L476RG`) and the display-required set is a
    ///     small explicit list, so a loose match is safe and necessary here.
    fn covers(&self, board: &Board, class: Class) -> bool {
        let markers = class.filename_markers();
        let config_needle = format!("chips/{}.yaml", board.yaml_stem);
        for (name, body) in &self.files {
            let is_class_file = markers.iter().any(|m| name.contains(m));
            if !is_class_file {
                continue;
            }
            let filename_alias = board
                .aliases
                .iter()
                .any(|a| name.contains(&a.to_lowercase()));
            let targets = match class {
                Class::Reset | Class::Exec => filename_alias || body.contains(&config_needle),
                Class::Display => {
                    filename_alias
                        || body.contains(&config_needle)
                        || board
                            .aliases
                            .iter()
                            .any(|a| body.contains(&a.to_lowercase()))
                }
            };
            if targets {
                return true;
            }
        }
        false
    }
}

fn collect_rs(dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_lowercase();
        let body = fs::read_to_string(&path).unwrap_or_default().to_lowercase();
        out.push((name, body));
    }
}

fn required_classes(board: &Board) -> Vec<Class> {
    let mut v = vec![Class::Reset, Class::Exec];
    if board.display_lab {
        v.push(Class::Display);
    }
    v
}

fn is_allowlisted(chip: &str, class: Class) -> bool {
    KNOWN_GAPS.iter().any(|(c, k)| *c == chip && *k == class)
}

#[test]
fn board_coverage_ratchet() {
    let corpus = Corpus::load();

    let mut hard_failures: Vec<String> = Vec::new(); // uncovered AND not allowlisted
    let mut stale_allowlist: Vec<String> = Vec::new(); // allowlisted BUT now covered
    let mut honored_gaps: Vec<String> = Vec::new(); // allowlisted AND still missing

    for board in SHIPPED {
        for class in required_classes(board) {
            let covered = corpus.covers(board, class);
            let allowlisted = is_allowlisted(board.chip, class);
            match (covered, allowlisted) {
                (false, false) => hard_failures.push(format!(
                    "  MISSING  {:<10} needs {}",
                    board.chip,
                    class.label()
                )),
                (false, true) => {
                    honored_gaps.push(format!("  KNOWN-GAP {:<10} {}", board.chip, class.label()))
                }
                (true, true) => stale_allowlist.push(format!(
                    "  STALE     {:<10} {} — now COVERED; delete its KNOWN_GAPS row",
                    board.chip,
                    class.label()
                )),
                (true, false) => {}
            }
        }
    }

    // Any (chip, class) in the allowlist that is not even a shipped requirement
    // is dead weight — catch typos / removed chips.
    let shipped_ids: BTreeSet<&str> = SHIPPED.iter().map(|b| b.chip).collect();
    for (chip, class) in KNOWN_GAPS {
        let required = SHIPPED
            .iter()
            .find(|b| b.chip == *chip)
            .map(|b| required_classes(b).contains(class))
            .unwrap_or(false);
        if !shipped_ids.contains(chip) {
            stale_allowlist.push(format!("  STALE     {chip} — not a shipped board"));
        } else if !required {
            stale_allowlist.push(format!(
                "  STALE     {chip} {} — not a required class for this board",
                class.label()
            ));
        }
    }

    if !honored_gaps.is_empty() {
        eprintln!(
            "board-coverage ratchet: {} known gap(s) tracked (shrink-only):\n{}",
            honored_gaps.len(),
            honored_gaps.join("\n")
        );
    }

    let mut msg = String::new();
    if !hard_failures.is_empty() {
        msg.push_str(&format!(
            "\nSHIPPED BOARD(S) LACK REQUIRED COVERAGE (add the convention test, do NOT \
             weaken the gate):\n{}\n",
            hard_failures.join("\n")
        ));
    }
    if !stale_allowlist.is_empty() {
        msg.push_str(&format!(
            "\nSTALE KNOWN_GAPS entries (coverage exists — delete the row so the ratchet \
             tightens):\n{}\n",
            stale_allowlist.join("\n")
        ));
    }
    assert!(msg.is_empty(), "{msg}");
}

/// A shipped chip descriptor must be represented in `SHIPPED`, so no board can
/// bypass the ratchet by simply not being listed. (Non-shipped / template /
/// in-progress descriptors are excluded explicitly.)
#[test]
fn every_shipped_descriptor_is_ratcheted() {
    // Chip descriptors that exist in configs/chips but are NOT in the canonical
    // shipped catalog (bundled-configs.ts). They are intentionally not ratcheted.
    const NOT_SHIPPED: &[&str] = &[
        "esp32",         // classic Xtensa, separate e2e lane, not a catalog board
        "esp32s3-zero",  // board variant of esp32s3 (covered by esp32s3)
        "stm32f401cdu6", // BlackPill variant of stm32f401 (covered by stm32f401)
        "stm32g474re",   // G4 peripheral models in progress, not shipped
        "stm32wb55",     // BLE not modelled, not shipped
        "stm32wba52",    // WBA early onboarding, not shipped
        "nrf52832",      // covered by nrf52840 family; not a catalog board
        "nrf5340",       // dual-core, not a shipped catalog board
        // Boots unmodified upstream Zephyr and has bus-level conformance +
        // survival coverage, but NOT the executing-fidelity class this gate
        // requires for SHIPPED: there is no walk-vs-scheduler differential and
        // no silicon oracle for its GRTC/IRQ path. Surviving a boot is not the
        // same as being right. Promote it only when that differential exists.
        "nrf54l15",
        // First Cortex-M7 chip; sim-derived (RM0468, no bench part). Has a
        // tier-1 fixture + io-smoke, but not yet a bundled-configs.ts catalog
        // board and no silicon oracle. Promote when it ships in the playground
        // catalog and gains an executing-fidelity differential.
        "stm32h735",
    ];
    // configs/chips id -> ratchet chip id (kw41z ships as mkw41z4.yaml).
    fn to_ratchet_id(stem: &str) -> &str {
        match stem {
            "mkw41z4" => "kw41z",
            other => other,
        }
    }

    let chips_dir = core_crate_root().join("../../configs/chips");
    let shipped_ids: BTreeSet<&str> = SHIPPED.iter().map(|b| b.chip).collect();
    let not_shipped: BTreeSet<&str> = NOT_SHIPPED.iter().copied().collect();

    let mut unaccounted = Vec::new();
    for entry in fs::read_dir(&chips_dir)
        .expect("read configs/chips")
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if stem.starts_with('_') || stem.starts_with("ci-fixture") {
            continue;
        }
        let id = to_ratchet_id(stem);
        if !shipped_ids.contains(id) && !not_shipped.contains(stem) {
            unaccounted.push(format!(
                "  {stem}: neither in SHIPPED nor in NOT_SHIPPED — decide and list it"
            ));
        }
    }
    assert!(
        unaccounted.is_empty(),
        "chip descriptor(s) escape the coverage ratchet:\n{}",
        unaccounted.join("\n")
    );
}
