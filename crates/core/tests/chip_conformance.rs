// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Standardized per-chip conformance scoreboard + ratchet.
//!
//! ONE uniform battery, run for EVERY chip, so coverage is comparable across the
//! fleet and can never silently regress. This sits *on top of* the existing
//! mechanisms rather than replacing them:
//!
//!   * **Estate** (all chips, always): the chip descriptor loads and every wired
//!     peripheral window is reachable (a read at its base faults nowhere).
//!   * **Registers vs silicon** (chips with a committed capture): the fraction of
//!     a real-silicon reset capture (`reg_oracle.json`) the sim reproduces. The
//!     deep per-register gate stays in `*_reset_conformance` / `register_coverage`;
//!     here we track the headline match% so it can't drop.
//!   * **Behavior** (chips with a golden firmware): whether a running-firmware
//!     gate exists (`firmware_survival` / `*_exec_oracle`), which boots real FW
//!     and asserts its register/IO effects.
//!
//! The board is written to `docs/coverage/chip-conformance.md`; the ratchet
//! baseline is `docs/coverage/chip-conformance.json`. A chip's estate must stay
//! green, its reg-match% may not fall, and a present behavior gate may not vanish.
//! Re-baseline (after a deliberate, explained change):
//!   UPDATE_CONFORMANCE_BASELINE=1 cargo test -p labwired-core --test chip_conformance -- --nocapture
//!
//! "Are the gates enough?" is now a number per chip on the board — and missing
//! coverage is a visible red cell, not a silent gap.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::path::PathBuf;

/// One chip's conformance inputs. `reset_oracle` and `behavior_gate` are `None`
/// until that coverage exists — the scoreboard then shows the gap.
struct ChipConf {
    name: &'static str,
    yaml: &'static str,
    /// Committed real-silicon reset capture (schema labwired-hw-oracle/*-regs).
    reset_oracle: Option<&'static str>,
    /// Name of the running-firmware gate that asserts this chip's behavior.
    behavior_gate: Option<&'static str>,
}

/// The fleet. Every chip with a descriptor MUST appear here (enforced below), so
/// a new chip can't be added without landing on the board.
const CHIPS: &[ChipConf] = &[
    ChipConf {
        name: "esp32c3",
        yaml: "configs/chips/esp32c3.yaml",
        reset_oracle: Some("scripts/hw-oracle/captures/esp32c3/20260611T161223Z/reg_oracle.json"),
        behavior_gate: Some("firmware_survival::test_esp32c3_demo_survival"),
    },
    ChipConf {
        name: "esp32",
        yaml: "configs/chips/esp32.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "esp32s3",
        yaml: "configs/chips/esp32s3.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "esp32s3-zero",
        yaml: "configs/chips/esp32s3-zero.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32f401cdu6",
        yaml: "configs/chips/stm32f401cdu6.yaml",
        reset_oracle: None,
        behavior_gate: Some("onboarding-stm32f401cdu6"),
    },
    ChipConf {
        name: "nrf52832",
        yaml: "configs/chips/nrf52832.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf52832_demo_survival"),
    },
    ChipConf {
        name: "nrf52840",
        yaml: "configs/chips/nrf52840.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nrf52840_demo_survival"),
    },
    ChipConf {
        name: "rp2040",
        yaml: "configs/chips/rp2040.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_rp2040_demo_survival"),
    },
    ChipConf {
        name: "stm32f103",
        yaml: "configs/chips/stm32f103.yaml",
        reset_oracle: None,
        behavior_gate: Some("stm32f1_exec_oracle"),
    },
    ChipConf {
        name: "stm32f401",
        yaml: "configs/chips/stm32f401.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32f401_blinky_survival"),
    },
    ChipConf {
        name: "stm32f407",
        yaml: "configs/chips/stm32f407.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_f407_smoke_survival"),
    },
    ChipConf {
        name: "stm32g474re",
        yaml: "configs/chips/stm32g474re.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32h563",
        yaml: "configs/chips/stm32h563.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_stm32h563_demo_survival"),
    },
    ChipConf {
        name: "stm32l073",
        yaml: "configs/chips/stm32l073.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_l073rz_smoke_survival"),
    },
    ChipConf {
        name: "stm32l476",
        yaml: "configs/chips/stm32l476.yaml",
        reset_oracle: None,
        behavior_gate: Some("firmware_survival::test_nucleo_l476rg_demo_survival"),
    },
    ChipConf {
        name: "stm32wb55",
        yaml: "configs/chips/stm32wb55.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
    ChipConf {
        name: "stm32wba52",
        yaml: "configs/chips/stm32wba52.yaml",
        reset_oracle: None,
        behavior_gate: None,
    },
];

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn dummy_manifest(path: &str) -> SystemManifest {
    SystemManifest {
        walk_deleted: false,
        schema_version: "1.0".to_string(),
        name: "chip-conformance".to_string(),
        chip: path.to_string(),
        external_devices: vec![],
        board_io: vec![],
        peripherals: vec![],
        memory_overrides: Default::default(),
    }
}

#[derive(Debug, Clone)]
struct Record {
    estate_ok: bool,
    peripherals: usize,
    reg_total: usize,
    reg_match: usize,
    behavior: bool,
}

/// Run the uniform battery for one chip.
fn measure(c: &ChipConf) -> Record {
    let abs = root(c.yaml);
    let abs_str = abs.to_string_lossy().to_string();
    let chip =
        ChipDescriptor::from_file(&abs).unwrap_or_else(|e| panic!("{}: load chip: {e}", c.name));
    let peripherals = chip.peripherals.len();
    let bus = SystemBus::from_config(&chip, &dummy_manifest(&abs_str))
        .unwrap_or_else(|e| panic!("{}: build bus: {e}", c.name));

    // Estate: every wired peripheral's base reads without a bus fault.
    let estate_ok = chip
        .peripherals
        .iter()
        .all(|p| bus.read_u32(p.base_address).is_ok());

    // Registers vs silicon: how much of the committed capture the sim reproduces.
    let (mut reg_total, mut reg_match) = (0usize, 0usize);
    if let Some(oracle) = c.reset_oracle {
        if let Ok(text) = std::fs::read_to_string(root(oracle)) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(blocks) = json.get("blocks").and_then(|b| b.as_object()) {
                    for block in blocks.values() {
                        if let Some(words) = block.get("words").and_then(|w| w.as_object()) {
                            for (addr, val) in words {
                                let a = parse_hex(addr);
                                let v = val.as_str().map(parse_hex32).unwrap_or(0);
                                reg_total += 1;
                                if matches!(bus.read_u32(a), Ok(got) if got == v) {
                                    reg_match += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Record {
        estate_ok,
        peripherals,
        reg_total,
        reg_match,
        behavior: c.behavior_gate.is_some(),
    }
}

fn parse_hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}
fn parse_hex32(s: &str) -> u32 {
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or(0)
}

/// A chip's conformance level: L0 estate, L1 +silicon-registers, L2 +behavior.
fn level(r: &Record) -> u8 {
    if !r.estate_ok {
        return 0;
    }
    let has_reg = r.reg_total > 0 && r.reg_match * 100 >= r.reg_total * 50;
    match (has_reg, r.behavior) {
        (true, true) => 2,
        (true, false) | (false, true) => 1,
        (false, false) => 0,
    }
}

#[test]
fn chip_conformance_ratchet() {
    // Every chip with a descriptor must be on the board.
    let configured: Vec<String> = std::fs::read_dir(root("configs/chips"))
        .expect("configs/chips")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".yaml"))
        .map(|n| n.trim_end_matches(".yaml").to_string())
        .filter(|n| !n.contains("ci-fixture"))
        .collect();
    for chip in &configured {
        assert!(
            CHIPS.iter().any(|c| c.name == chip),
            "chip '{chip}' has a config but is not in the conformance board — add it to CHIPS"
        );
    }

    let mut rows = Vec::new();
    let mut board = String::from(
        "# Chip Conformance Scoreboard\n\n\
         Generated by `chip_conformance_ratchet`. L0 estate · L1 +registers-vs-silicon · L2 +behavior.\n\n\
         | Chip | Level | Estate | Peripherals | Reg match (silicon) | Behavior gate |\n\
         |------|-------|--------|-------------|---------------------|---------------|\n",
    );
    for c in CHIPS {
        let r = measure(c);
        let lvl = level(&r);
        let reg = if r.reg_total > 0 {
            format!(
                "{}/{} ({}%)",
                r.reg_match,
                r.reg_total,
                r.reg_match * 100 / r.reg_total
            )
        } else {
            "—".to_string()
        };
        let beh = c.behavior_gate.unwrap_or("—");
        board.push_str(&format!(
            "| {} | **L{}** | {} | {} | {} | {} |\n",
            c.name,
            lvl,
            if r.estate_ok { "✓" } else { "✗" },
            r.peripherals,
            reg,
            beh,
        ));
        rows.push((c.name.to_string(), lvl, r));
    }

    std::fs::write(root("docs/coverage/chip-conformance.md"), &board).ok();

    // Ratchet against the committed baseline: estate may not break, level may not
    // drop, reg-match count may not fall.
    let baseline_path = root("docs/coverage/chip-conformance.json");
    let current: serde_json::Value = serde_json::json!(rows
        .iter()
        .map(|(name, lvl, r)| {
            serde_json::json!({"name": name, "level": lvl, "reg_match": r.reg_match, "behavior": r.behavior})
        })
        .collect::<Vec<_>>());

    if std::env::var("UPDATE_CONFORMANCE_BASELINE").is_ok() {
        std::fs::write(
            &baseline_path,
            serde_json::to_string_pretty(&current).unwrap(),
        )
        .expect("write baseline");
        println!("updated conformance baseline: {}", baseline_path.display());
        return;
    }

    let baseline: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path).unwrap_or_else(|_| {
            panic!(
                "missing {}; create it with UPDATE_CONFORMANCE_BASELINE=1",
                baseline_path.display()
            )
        }),
    )
    .expect("parse baseline");

    let mut failures = Vec::new();
    for (name, lvl, r) in &rows {
        let base = baseline.as_array().and_then(|a| {
            a.iter()
                .find(|b| b.get("name").and_then(|n| n.as_str()) == Some(name))
        });
        let Some(base) = base else { continue };
        let base_lvl = base.get("level").and_then(|l| l.as_u64()).unwrap_or(0) as u8;
        let base_match = base.get("reg_match").and_then(|m| m.as_u64()).unwrap_or(0) as usize;
        if *lvl < base_lvl {
            failures.push(format!("  {name}: level L{lvl} < baseline L{base_lvl}"));
        }
        if r.reg_match < base_match {
            failures.push(format!(
                "  {name}: reg match {} < baseline {base_match}",
                r.reg_match
            ));
        }
        if !r.estate_ok {
            failures.push(format!(
                "  {name}: estate broken (a peripheral window faults)"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "chip conformance regressed ({} issue(s)):\n{}\n(intentional? re-baseline with UPDATE_CONFORMANCE_BASELINE=1)",
        failures.len(),
        failures.join("\n")
    );
}
