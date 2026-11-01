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
//! For every SHIPPED modeled chip (declared-only platforms are explicitly
//! classified below and receive no board-coverage credit):
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

use labwired_config::{Arch, ChipDescriptor};
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
        chip: "stm32u5a5zj",
        aliases: &["stm32u5a5zj", "u5a5", "nucleo-u5a5zj"],
        yaml_stem: "stm32u5a5zj",
        display_lab: false,
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
    Board {
        chip: "atmega328p",
        aliases: &["atmega328p", "nano", "avr"],
        yaml_stem: "atmega328p",
        display_lab: false,
    },
    // Bare 64-pin ATmega1281 shipped in the Playground picker. Its committed
    // AVR-GCC firmware is exercised by both the smoke and execution differential.
    Board {
        chip: "atmega1281",
        aliases: &["atmega1281"],
        yaml_stem: "atmega1281",
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

/// Platform declarations are not modeled boards or coverage-gap waivers.
/// Promotion to a modeled board must remove this entry and classify coverage.
const DECLARED_ONLY: &[(&str, &str)] = &[
    (
        "rv1103",
        "CPU/memory declaration only; no RV1103 firmware/platform CI gate. Peripherals, boot chain and Linux/BSP execution are unimplemented. Generic Cortex-A7 CPU subset tests supply no RV1103 board coverage.",
    ),
    (
        "xck26-rpu",
        "CPU/memory declaration; the RPU platform (GIC, Cadence UART, TTC) exists only on the native qemu-arm-system Cortex-R5F path, gated in core-qemu-arm (kv260_r5_zephyr, examples/kv260-r5), not the default board CI. No GPIO/I2C/SPI or other PS peripherals are modelled, so there is no board I/O coverage to classify.",
    ),
];

fn is_declared_only_platform(stem: &str, chip: &ChipDescriptor) -> bool {
    let core = match stem {
        "rv1103" => "cortex-a7",
        "xck26-rpu" => "cortex-r5f",
        _ => return false,
    };
    chip.name == stem
        && chip.arch == Arch::Arm
        && chip.core.as_deref() == Some(core)
        && chip.peripherals.is_empty()
        && chip.flash.size == 0
        && chip.flash.base == 0
}

#[test]
fn declared_only_classification_rejects_other_or_modeled_platforms() {
    assert_eq!(DECLARED_ONLY.len(), 2);
    assert_eq!(DECLARED_ONLY[0].0, "rv1103");
    assert_eq!(DECLARED_ONLY[1].0, "xck26-rpu");
    let rpu =
        ChipDescriptor::from_file(core_crate_root().join("../../configs/chips/xck26-rpu.yaml"))
            .unwrap();
    assert!(is_declared_only_platform("xck26-rpu", &rpu));
    assert!(!is_declared_only_platform("rv1103", &rpu));
    assert!(!SHIPPED.iter().any(|board| board.yaml_stem == "rv1103"));
    let chip = ChipDescriptor::from_file(core_crate_root().join("../../configs/chips/rv1103.yaml"))
        .unwrap();
    assert!(is_declared_only_platform("rv1103", &chip));
    assert!(!is_declared_only_platform("another-prototype", &chip));
    let mut changed = chip.clone();
    changed.name = "another-prototype".into();
    assert!(!is_declared_only_platform("rv1103", &changed));
    changed = chip.clone();
    changed.arch = Arch::RiscV;
    assert!(!is_declared_only_platform("rv1103", &changed));
    changed = chip.clone();
    changed.core = Some("cortex-m7".into());
    assert!(!is_declared_only_platform("rv1103", &changed));
    changed = chip.clone();
    changed.flash.size = 4096;
    assert!(!is_declared_only_platform("rv1103", &changed));
    changed = chip.clone();
    changed.flash.base = 0x2000_0000;
    assert!(!is_declared_only_platform("rv1103", &changed));
    changed = chip;
    changed.peripherals =
        ChipDescriptor::from_file(core_crate_root().join("../../configs/chips/stm32f103.yaml"))
            .unwrap()
            .peripherals;
    assert!(!is_declared_only_platform("rv1103", &changed));
}

#[test]
fn generic_cpu_tests_do_not_supply_rv1103_board_coverage() {
    let board = Board {
        chip: "rv1103",
        aliases: &["rv1103"],
        yaml_stem: "rv1103",
        display_lab: false,
    };
    let corpus = Corpus {
        files: vec![
            (
                "cpu_trace_conformance.rs".into(),
                "generic cortex-a7 cpu subset".into(),
            ),
            (
                "cortex_a7_golden.rs".into(),
                "generic cortex-a7 cpu subset".into(),
            ),
        ],
    };
    assert_eq!(required_classes(&board), vec![Class::Reset, Class::Exec]);
    assert!(!corpus.covers(&board, Class::Reset));
    assert!(!corpus.covers(&board, Class::Exec));
    assert!(!is_allowlisted(board.chip, Class::Reset));
    assert!(!is_allowlisted(board.chip, Class::Exec));
}

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

/// A shipped modeled chip descriptor must be represented in `SHIPPED`, so no board can
/// bypass the ratchet by simply not being listed. (Non-shipped / template /
/// in-progress descriptors are excluded explicitly.)
#[test]
fn every_shipped_descriptor_is_ratcheted() {
    // Chip descriptors that exist in configs/chips but are NOT in the canonical
    // shipped catalog (bundled-configs.ts). They are intentionally not ratcheted.
    const NOT_SHIPPED: &[&str] = &[
        // ATmega2560 (Arduino Mega 2560) on the 328P AVR model. Has a chip
        // descriptor, a system manifest, a gated sim-validate example with an
        // io-smoke and the avr_mega2560_io integration tests. It is not a
        // bundled-configs.ts catalog board yet (the superproject board is a
        // separate PR) and it has no executing-fidelity differential of its own.
        // Promote when the catalog board lands AND a differential exists.
        "atmega2560",
        // 64-pin ATmega2561 on the same public AVR core. Has a descriptor,
        // a system manifest, an example with a deterministic io-smoke, a
        // committed fixture and a chip-conformance behavior gate — but no
        // bundled-configs.ts catalog board and no executing-fidelity
        // differential of its own. Promote when both exist.
        "atmega2561",
        // First Microchip part in the engine. Has a chip descriptor, an
        // io-smoke executed by the coverage matrix and the strict-onboarding
        // gate, and a peripheral-estate test — but it is NOT a
        // bundled-configs.ts catalog board, has no silicon oracle (no SAM D21
        // bench part has ever been diffed over SWD), and carries no
        // executing-fidelity differential of its own. Same bar as rp2350 and
        // stm32f411ceu6 below. Promote when the catalog registration lands and
        // a real board is benched.
        "atsamd21g18a",
        // Exact A-variant SAM D21 orderable geometries. They reuse the public
        // G18A engine and add no peripheral/clock models of their own; their
        // geometry is pinned by `atsamd21_variant_geometry`, but they have no
        // catalog board, no silicon oracle and no executing-fidelity
        // differential. Promote only when a bench part and a differential
        // exist, exactly like atsamd21g18a above.
        "atsamd21e15a",
        "atsamd21e16a",
        "atsamd21e17a",
        "atsamd21e18a",
        "atsamd21g15a",
        "atsamd21g16a",
        "atsamd21g17a",
        "atsamd21j15a",
        "atsamd21j16a",
        "atsamd21j17a",
        "atsamd21j18a",
        "esp32",         // classic Xtensa, separate e2e lane, not a catalog board
        "esp32s3-zero",  // board variant of esp32s3 (covered by esp32s3)
        "stm32f401cdu6", // BlackPill variant of stm32f401 (covered by stm32f401)
        // Exact STM32 order-code geometries for the F103/F401/F407 engines.
        // These are core chip descriptors, not Playground boards in
        // bundled-configs.ts; their family behavior is shared with the three
        // catalog boards above. Per-part geometry is guarded by
        // stm32_variant_geometry, and the catalog CI smoke executes each exact
        // descriptor, but none has its own silicon differential.
        "stm32f103c4",
        "stm32f103c6",
        "stm32f103r4",
        "stm32f103r4h",
        "stm32f103r6",
        "stm32f103r6h",
        "stm32f103rb",
        "stm32f103t4",
        "stm32f103t6",
        "stm32f401cb",
        "stm32f401cc",
        "stm32f401cd",
        "stm32f401ce",
        "stm32f401rb",
        "stm32f401rc",
        "stm32f401rd",
        "stm32f401re",
        "stm32f401vb",
        "stm32f401vc",
        "stm32f401vd",
        "stm32f401ve",
        "stm32f407ie",
        "stm32f407ig",
        "stm32f407ve",
        "stm32f407vg",
        "stm32f407ze",
        "stm32f407zg",
        // First 16-bit PIC/dsPIC target, M0 bring-up (boot + UART1 TX). Has a
        // chip descriptor, an example with an io-smoke, unit + integration
        // tests — but not a bundled-configs.ts catalog board and no silicon
        // oracle. Promote when peripheral milestones land.
        "dspic33fj32mc204",
        // 28-pin sibling of dspic33fj32mc204, M0 bring-up on the shared
        // PIC24 core. Same rationale: no bundled-configs.ts board and no
        // silicon oracle yet.
        "dspic33fj32mc202",
        // First PIC18 part on the public PIC18 core: L1 boot + EUSART TX +
        // PORTB latch from real SDCC firmware. No bundled-configs.ts board,
        // no silicon oracle and no executing-fidelity differential yet.
        "pic18f4520",
        // First PIC16 part on the new PIC16 classic mid-range core: L1 boot +
        // USART TX + PORTB latch from real SDCC firmware. No bundled-configs.ts
        // board, no silicon oracle and no executing-fidelity differential yet.
        "pic16f877a",
        // First enhanced mid-range PIC16 part: L1 boot + EUSART TX + LATA +
        // INT/TMR0 interrupts from real SDCC firmware. No bundled-configs.ts
        // board, no silicon oracle and no executing-fidelity differential yet.
        "pic16f1829",
        // PIC24F KA family: six exact part descriptors plus per-part XC16 UART
        // smokes on the public PIC24 core. The modelled slice is package GPIO
        // and UART1 only; no bundled-configs.ts catalog board, no silicon
        // oracle and no executing-fidelity differential. Promote when the
        // catalog and a differential exist.
        "pic24f04ka200",
        "pic24f04ka201",
        "pic24f08ka101",
        "pic24f08ka102",
        "pic24f16ka101",
        "pic24f16ka102",
        // First RISC-V soft-core SoC in the engine (LiteX VexRiscv, RV32IM).
        // Has a chip descriptor, a system manifest, an example with an
        // io-smoke, a committed smoke fixture and a firmware-survival gate —
        // but not a bundled-configs.ts catalog board, no silicon capture (no
        // bench bitstream diff), and no executing-fidelity differential of
        // its own. Promote when the browser board lands AND a differential
        // exists.
        "litex-vexriscv",
        // Digilent Arty A7-35T on the LiteX VexRiscv SoC: chip + system + example
        // with an io-smoke, a committed fixture and a survival gate, but no
        // bundled-configs.ts catalog board, no bitstream capture and no
        // executing-fidelity differential of its own. Same bar as litex-vexriscv.
        "arty-litex-vexriscv",
        // Vega board (RI5CY): chip + system + example with an io-smoke, a
        // committed fixture and a survival gate; no catalog board, no silicon
        // capture and no executing-fidelity differential.
        "vegaboard-ri5cy",
        // First SAM4S part (SAM4S Xplained): chip + system + example with an
        // io-smoke, a committed fixture and a survival gate; no catalog board,
        // no silicon capture and no executing-fidelity differential.
        "sam4s16c",
        // First SAM E70 part (SAM E70 Xplained): chip + system + example with
        // an io-smoke, a committed fixture and a survival gate; no catalog
        // board, no silicon capture and no executing-fidelity differential.
        "same70q21b",
        // First S32K3 part (S32K388EVB-Q289): chip + system + example with
        // an io-smoke, a committed fixture and a survival gate; no catalog
        // board, no silicon capture and no executing-fidelity differential.
        "s32k388",
        // First i.MX RT500 part (MIMXRT595-EVK): chip + system + example with
        // an io-smoke, a committed fixture and a survival gate; no catalog
        // board, no silicon capture and no executing-fidelity differential.
        "mimxrt595s",
        // First TI CC2538 part (CC2538DK): chip + system + example with an
        // io-smoke, a committed fixture and a survival gate; no catalog board,
        // no silicon capture and no executing-fidelity differential.
        "cc2538sf53",
        // First RV64 part (PolarFire SoC Discovery Kit, MPFS095T U54 hart):
        // chip + system + example with stock Zephyr hello/sync/kernel-common,
        // committed fixtures and a survival gate; no catalog board, no silicon
        // capture and no executing-fidelity differential.
        "mpfs095t",
        // First RX part (EK-RX261, R5F52618BGFP, RXv3): chip + system +
        // example with stock Zephyr v4.3.0 hello/sync/kernel-common,
        // committed fixtures and a survival gate; no catalog board, no silicon
        // capture and no executing-fidelity differential.
        "r5f52618bgfp",
        // First RA6M5 part (EK-RA6M5): chip + system + example with an
        // io-smoke, a committed fixture and a survival gate; no catalog board,
        // no silicon capture and no executing-fidelity differential.
        "r7fa6m5bh",
        // First RA8 part (EK-RA8M1, Cortex-M85): chip + system + example with
        // an io-smoke, a committed fixture and a survival gate; no catalog
        // board, no silicon capture and no executing-fidelity differential.
        "r7fa8m1ah",
        // First Ambiq part (Apollo4 Plus EVB): chip + system + example with an
        // io-smoke, a committed fixture and a survival gate; no catalog board,
        // no silicon capture and no executing-fidelity differential.
        "amap42kp",
        // First STM32L5 part (NUCLEO-L552ZE-Q): chip + system + example with an
        // io-smoke, a committed fixture and a survival gate; no catalog board,
        // no silicon capture and no executing-fidelity differential.
        "stm32l552",
        // FRDM-K64F UART-only L1: chip + system + example with an io-smoke, a
        // committed fixture and a survival gate; no catalog board, no silicon
        // capture and no executing-fidelity differential. ENET/TCP deferred.
        "mk64f12",
        // First STM32F0 part (STM32F072B-DISCO): chip + system + example with
        // an io-smoke, a committed fixture and a survival gate; no catalog
        // board, no silicon capture and no executing-fidelity differential.
        "stm32f072",
        // Descriptor-level derivative of the F401 lane (shared F4 profiles,
        // DEV_ID 0x413), smoke-validated via the feather-f405 example only:
        // no catalog board, no silicon oracle, no executing-fidelity
        // differential. Promote when a real F405 board ships and is benched.
        "stm32f405",
        // M7 on the F4-profile shared IP (F7 I2C = v2/stm32l4 profile),
        // smoke-validated via the nucleo-f767zi example only: no catalog
        // board, no silicon oracle, and the F7-only RCC registers
        // (PLLSAI/DCKCFGR) are unmodeled. Promote when benched on F767
        // silicon and the clock-tree gap closes.
        "stm32f767",
        // Dual M33 on the RP2040-model shared IP with an rp2350 clkrst
        // address-map profile. Smoke-validated via examples/pico2 only:
        // no catalog board, no silicon oracle, no TrustZone/bootrom/PIO
        // v2. Promote when benched on Pico 2 silicon.
        "rp2350",
        // WeAct F411 Black Pill. Has a tier-1 fixture + io-smoke and rides the
        // shared stm32f4 peripheral models, but it is not a bundled-configs.ts
        // catalog board and carries NO executing-fidelity test of its own (no
        // walk-vs-scheduler differential, no silicon oracle — there is no F411
        // bench part). Promote to SHIPPED only when both of those exist.
        "stm32f411ceu6",
        "stm32g474re", // G4 peripheral models in progress, not shipped
        "stm32wb55",   // BLE not modelled, not shipped
        "stm32wba52",  // WBA early onboarding, not shipped
        "nrf52832",    // covered by nrf52840 family; not a catalog board
        // micro:bit v2 target. Shares the nRF52840 family's peripheral models
        // and carries a UARTE EasyDMA smoke survival case + a config-build
        // gate, but has no silicon oracle and no executing-fidelity
        // differential, and is not a bundled-configs.ts catalog board.
        "nrf52833",
        "nrf5340", // dual-core, not a shipped catalog board
        // micro:bit V1 / Calliope mini 1, S110 application region only. A
        // legacy-UART smoke survival case and the SoftDevice HLE (whose
        // official-image runs are not a gate here), but no silicon oracle, no
        // executing-fidelity differential, and not a bundled-configs.ts
        // catalog board. Promote when those exist.
        "nrf51822",
        // Boots unmodified upstream Zephyr and has bus-level conformance +
        // survival coverage, but NOT the executing-fidelity class this gate
        // requires for SHIPPED: there is no walk-vs-scheduler differential and
        // no silicon oracle for its GRTC/IRQ path. Surviving a boot is not the
        // same as being right. Promote it only when that differential exists.
        "nrf54l15",
        // Same standing as its sibling above, for the same reason. The
        // nrf54lm20a-snake lab drives UARTE20, SPIM22, the RM67162 panel and
        // four buttons across two GPIO ports, with display-region evidence and
        // a verified-discriminating negative control -- but that is BUS-level
        // conformance and survival, not the executing-fidelity class this gate
        // requires: there is no walk-vs-scheduler differential and no silicon
        // oracle for its GRTC/IRQ path. It is also not yet a bundled-configs.ts
        // catalog board. Promote when both of those change.
        "nrf54lm20a",
        // First Cortex-M7 chip; sim-derived (RM0468, no bench part). Has a
        // tier-1 fixture + io-smoke, but not yet a bundled-configs.ts catalog
        // board and no silicon oracle. Promote when it ships in the playground
        // catalog and gains an executing-fidelity differential.
        "stm32h735",
        // Silicon Labs EFR32MG26 (Series-2). L1 smoke, validated via the
        // brd2709a example (uart + io scripts) and bench-proven on the
        // physical board (VCOM banner, 2026-08-18): no catalog board, no
        // silicon register oracle, CMU/TIMER0 are stubs. Promote when an
        // executing-fidelity differential exists.
        "efr32mg26",
        // The five maker-five descriptors (#1124). All five carry a UART/LED
        // smoke survival case and a config-build gate, and nothing else: no
        // executing-fidelity differential (no walk-vs-scheduler, no silicon
        // oracle — none of these parts has been benched). They are also NOT
        // bundled-configs.ts catalog boards yet; the Playground registration
        // is an open superproject PR. Same bar as atsamd21g18a / f411ceu6
        // above: promote when the catalog lands AND a differential exists.
        //
        // `atsamd21` is the Nano 33 IoT descriptor (the Zero's own
        // `atsamd21g18a.yaml` is already listed above; sharing the name made
        // the Playground pin-map generator emit a duplicate const, so the
        // Nano yaml is named for the part family it reuses).
        "atsamd21",
        "atsamd51",
        // Adafruit PyBadge application layout: the atsamd51 descriptor with
        // reset_vector_offset 0x4000 (UF2 bootloader resident below). Board
        // variant of atsamd51, carrying only a config-build + boot-state gate
        // (pybadge_config.rs): no silicon oracle, no executing-fidelity
        // differential, not a bundled-configs.ts catalog board.
        "atsamd51-pybadge",
        "ra4m1",
        "imxrt1064",
        "stm32f746",
        "stm32u575", // First U5 part; sim-validated, no bench silicon capture yet
        // NUCLEO-U545RE-Q: U575-family derivative (drift-guarded against
        // stm32u575.yaml). Survival + machine-run pin tests + SVD conformance,
        // but no silicon oracle and no executing-fidelity differential.
        "stm32u545",
        // First STM32G0 part. L1 smoke: UART/LED survival case, config-build
        // gate, and a register-vs-SVD measurement — but no silicon oracle (no
        // bench NUCLEO-G071RB), no executing-fidelity differential, and no
        // bundled-configs.ts catalog entry. Promote when all three exist.
        "stm32g071",
        // G0B1RE firmware slice: USART1, TIM2, EXTI, and three GPIO ports.
        // The descriptor says it is not a full-chip validation claim. No
        // silicon oracle, no executing-fidelity differential, and no
        // bundled-configs.ts catalog entry. Same bar as stm32g071 above.
        "stm32g0b1re",
        // ESP32-C6 HP core. L1 smoke: UART0 console reaches the capture sink
        // through its own memory map (flash @ 0x4200_0000, HP SRAM @
        // 0x4080_0000) and its own clock/reset block (PCR); PCR gates and the
        // interrupt matrix are register-backed stubs, LP core and radios are
        // out of scope. No executing-fidelity differential and no silicon
        // oracle — the C3's capture does NOT transfer (different map and
        // clock/reset blocks; UART IRQ source 43 vs 21). Not a
        // bundled-configs.ts catalog board. Same bar as the maker-five above:
        // promote when the catalog lands AND a differential exists.
        "esp32c6",
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
    let mut declared_seen = BTreeSet::new();
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
        if let Some((_, reason)) = DECLARED_ONLY.iter().find(|(name, _)| *name == stem) {
            assert!(
                !shipped_ids.contains(id) && !not_shipped.contains(stem),
                "{stem}: declaration must not overlap modeled board classifications"
            );
            let chip = ChipDescriptor::from_file(&path).expect("load declared platform");
            assert!(
                is_declared_only_platform(stem, &chip),
                "{stem}: platform no longer matches the declared-only prototype; classify its board coverage explicitly"
            );
            assert!(!reason.is_empty());
            declared_seen.insert(stem.to_string());
            eprintln!("board-coverage ratchet: DECLARED ONLY {stem}: {reason}");
            continue;
        }
        if !shipped_ids.contains(id) && !not_shipped.contains(stem) {
            unaccounted.push(format!(
                "  {stem}: neither in SHIPPED, NOT_SHIPPED nor validated DECLARED_ONLY — decide and list it"
            ));
        }
    }
    assert_eq!(
        declared_seen,
        DECLARED_ONLY
            .iter()
            .map(|(name, _)| name.to_string())
            .collect(),
        "stale declared-only classification: every entry must have its descriptor"
    );
    assert!(
        unaccounted.is_empty(),
        "chip descriptor(s) escape the coverage ratchet:\n{}",
        unaccounted.join("\n")
    );
}
