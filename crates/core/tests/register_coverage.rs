// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Register-modeling coverage vs the vendor SVD.
//!
//! For each supported chip with an in-tree SVD, this enumerates every register
//! the datasheet defines (via `svd-ingestor`) and probes the simulator's bus to
//! measure how many are actually modeled. It runs in CI as a **ratchet gate**
//! (`register_coverage_ratchet`): a chip's modeled count may never regress.
//! Chips whose newest-silicon SVDs aren't public yet are listed "SVD pending" —
//! and that is the *only* way to be pending. Everything else that used to make a
//! chip quietly unmeasurable (an unreadable SVD, an unparseable SVD, a peripheral
//! the ingestor chokes on, a chip absent from the baseline, an arch with no probe
//! path) is now a hard failure. Each of those was a way for this gate to pass
//! while measuring nothing.
//!
//! Per register we record three signals from the live bus:
//!   * `mapped` — a read succeeds (the address lands in a modeled peripheral)
//!   * `reset_ok` — the read value equals the SVD reset value
//!   * `responsive` — writing 0xFFFF_FFFF then 0 yields different read-backs,
//!     i.e. the register stores state (definitive proof of modeling)
//!
//! Headline `modeled` is the conservative union: `responsive || (reset_ok &&
//! reset != 0)`. It under-counts write-only and read-only-reset-0 registers
//! that are modeled but indistinguishable from an unhandled-offset default, so
//! treat it as a lower bound; `mapped` is the upper bound.
//!
//! It also under-counts **write-protected** registers whose reset value is 0:
//! the probe writes without any unlock sequence, so a register that correctly
//! refuses the write (e.g. IWDG_PR after #199 gated PR/RLR on the 0x5555 key,
//! RM0008 §19.4) reads back its reset 0 and looks unresponsive — even though it
//! is faithfully modeled. A fidelity fix that adds such gating therefore *lowers*
//! this proxy by design; re-baseline (see below), it is not a coverage loss.

use labwired_config::{Arch, ChipDescriptor};
use labwired_core::bus::SystemBus;
use labwired_core::{system, Bus, Machine};
use std::path::PathBuf;

/// All supported chips: (name, chip yaml, optional in-tree SVD).
///
/// `None` = no public vendor SVD available yet (the newest STM32 parts);
/// those are listed as "SVD pending" rather than silently dropped.
type ChipEntry = (&'static str, &'static str, Option<&'static str>);
const CHIPS: &[ChipEntry] = &[
    (
        "esp32",
        "configs/chips/esp32.yaml",
        Some("tests/fixtures/real_world/esp32.svd"),
    ),
    (
        "esp32c3",
        "configs/chips/esp32c3.yaml",
        Some("tests/fixtures/real_world/esp32c3.svd"),
    ),
    (
        "esp32s3",
        "configs/chips/esp32s3.yaml",
        Some("tests/fixtures/svd/esp32s3.svd"),
    ),
    (
        "nrf52832",
        "configs/chips/nrf52832.yaml",
        Some("tests/fixtures/real_world/nrf52832.svd"),
    ),
    (
        "nrf52840",
        "configs/chips/nrf52840.yaml",
        Some("tests/fixtures/real_world/nrf52840.svd"),
    ),
    (
        "rp2040",
        "configs/chips/rp2040.yaml",
        Some("tests/fixtures/real_world/rp2040.svd"),
    ),
    (
        "stm32f103",
        "configs/chips/stm32f103.yaml",
        Some("tests/fixtures/real_world/stm32f103.svd"),
    ),
    (
        "stm32f401",
        "configs/chips/stm32f401.yaml",
        Some("tests/fixtures/real_world/stm32f401.svd"),
    ),
    (
        "stm32f407",
        "configs/chips/stm32f407.yaml",
        Some("tests/fixtures/real_world/stm32f407.svd"),
    ),
    (
        "stm32f411ceu6",
        "configs/chips/stm32f411ceu6.yaml",
        Some("tests/fixtures/real_world/stm32f411.svd"),
    ),
    (
        "stm32g474re",
        "configs/chips/stm32g474re.yaml",
        Some("tests/fixtures/real_world/stm32g474.svd"),
    ),
    (
        "stm32h563",
        "configs/chips/stm32h563.yaml",
        Some("tests/fixtures/real_world/stm32h563.svd"),
    ),
    (
        "stm32l073",
        "configs/chips/stm32l073.yaml",
        Some("tests/fixtures/real_world/stm32l073.svd"),
    ),
    (
        "stm32l476",
        "configs/chips/stm32l476.yaml",
        Some("tests/fixtures/real_world/stm32l476.svd"),
    ),
    (
        "stm32wb55",
        "configs/chips/stm32wb55.yaml",
        Some("tests/fixtures/real_world/stm32wb55.svd"),
    ),
    (
        "stm32wba52",
        "configs/chips/stm32wba52.yaml",
        Some("tests/fixtures/real_world/stm32wba52.svd"),
    ),
];

/// Repo root (core/), resolved from this crate's manifest dir (core/crates/core).
fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn dummy_manifest(path: &str) -> labwired_config::SystemManifest {
    labwired_config::SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "coverage".to_string(),
        chip: path.to_string(),
        cpu_hz: None,
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    }
}

struct Probe {
    mapped: bool,
    reset_ok: bool,
    responsive: bool,
}

fn probe_register(bus: &mut SystemBus, addr: u64, reset: u32) -> Probe {
    let sim = bus.read_u32(addr);
    let mapped = sim.is_ok();
    let reset_ok = matches!(sim, Ok(v) if v == reset);
    let _ = bus.write_u32(addr, 0xFFFF_FFFF);
    let r1 = bus.read_u32(addr).ok();
    let _ = bus.write_u32(addr, 0x0000_0000);
    let r2 = bus.read_u32(addr).ok();
    let responsive = matches!((r1, r2), (Some(a), Some(b)) if a != b);
    Probe {
        mapped,
        reset_ok,
        responsive,
    }
}

/// Enumerate every SVD register as (absolute address, reset value).
///
/// Every failure here is **fatal**. This used to swallow three of them:
///
///   * an unreadable SVD file (`.ok()?`),
///   * an unparseable SVD (`.ok()?`),
///   * a peripheral the ingestor could not process (`Err(_) => continue`).
///
/// The first two turned into `measure_chip() == None`, which the ratchet loop
/// then skipped — delete an SVD and that chip's gate vanished *green*. The third
/// was worse than a skip: the dropped peripheral's registers left BOTH the
/// numerator and the denominator, so `modeled/total` could *rise* while real
/// coverage fell, and the ratchet (which compares absolute `modeled` counts)
/// would only notice if the drop happened to cross the baseline.
fn svd_registers(svd_path: &str) -> Vec<(u64, u32)> {
    let abs = root(svd_path);
    let xml = std::fs::read_to_string(&abs)
        .unwrap_or_else(|e| panic!("read SVD {}: {e}\n{}", abs.display(), SVD_FATAL_NOTE));
    let device = svd_ingestor::parse_svd(&xml)
        .unwrap_or_else(|e| panic!("parse SVD {}: {e}\n{}", abs.display(), SVD_FATAL_NOTE));
    let mut out = Vec::new();
    for peripheral in &device.peripherals {
        let base = peripheral.base_address;
        let desc = svd_ingestor::process_peripheral(&device, peripheral).unwrap_or_else(|e| {
            panic!(
                "{}: peripheral `{}` (base 0x{base:08x}) cannot be processed: {e}\n\
                 Skipping it would delete its registers from BOTH sides of the coverage \
                 fraction, so the percentage could rise while coverage fell. Fix the \
                 ingestor or the SVD; do not drop the peripheral.",
                abs.display(),
                peripheral.name
            )
        });
        for reg in &desc.registers {
            out.push((base + reg.address_offset, reg.reset_value));
        }
    }
    assert!(
        !out.is_empty(),
        "{}: parsed but yielded zero registers — the ratchet would then compare 0 >= 0 \
         forever.\n{}",
        abs.display(),
        SVD_FATAL_NOTE
    );
    out
}

const SVD_FATAL_NOTE: &str = "This is a ratchet input, not an optional extra: a chip whose SVD \
    cannot be read used to be recorded as \"SVD pending\" and skipped, so its coverage gate \
    disappeared silently and green. Restore the SVD, or delete the chip from CHIPS *and* from \
    docs/coverage/register-modeling.json in the same commit.";

fn probe_all(bus: &mut SystemBus, regs: &[(u64, u32)]) -> (usize, usize, usize) {
    let (mut mapped, mut reset_ok, mut modeled) = (0usize, 0usize, 0usize);
    for &(addr, reset) in regs {
        let p = probe_register(bus, addr, reset);
        if p.mapped {
            mapped += 1;
        }
        if p.reset_ok {
            reset_ok += 1;
        }
        if p.responsive || (p.reset_ok && reset != 0) {
            modeled += 1;
        }
    }
    (mapped, reset_ok, modeled)
}

/// One chip's measured coverage: (total SVD registers, mapped, reset_ok,
/// modeled). Panics rather than returning "unmeasured" — see `svd_registers`.
fn measure_chip(name: &str, yaml: &str, svd: &str) -> (usize, usize, usize, usize) {
    let regs = svd_registers(svd);
    let total = regs.len();
    let chip = ChipDescriptor::from_file(root(yaml)).expect("chip yaml");
    let mut bus = SystemBus::from_config(&chip, &dummy_manifest(yaml)).expect("bus");
    let (mapped, reset_ok, modeled) = match chip.arch {
        Arch::Arm => {
            let (cpu, _nvic) = system::cortex_m::configure_cortex_m(&mut bus);
            let mut m = Machine::new(cpu, bus);
            // Measure *modeling*, not the current runtime clock state. Out of
            // reset, RCC-clock-gated peripherals (STM32F1/L4) read back 0 and
            // ignore writes — silicon-accurate, but it makes a gated yet fully
            // modeled register look unresponsive to this probe. Bypass gating so
            // the count reflects whether the register is modeled, independent of
            // whether firmware happens to have clocked it. (Pre-setting the RCC
            // enable bits wouldn't work: the probe writes 0 to every register,
            // including the RCC enable registers, re-gating later peripherals.)
            m.bus.set_clock_gating_bypass(true);
            probe_all(&mut m.bus, &regs)
        }
        Arch::RiscV => {
            let cpu = system::riscv::configure_riscv(&mut bus);
            let mut m = Machine::new(cpu, bus);
            m.bus.set_clock_gating_bypass(true);
            // Same reasoning as the clock-gate bypass above, for the ESP32-C3
            // PMS: this probe writes 0xFFFF_FFFF to every register in ascending
            // offset order, which SETS the PMS lock bits and then scores the 13
            // registers behind them (and the hardware-owned violation-status
            // words) unresponsive. That would report a coverage regression for
            // registers that became MORE modelled, not less.
            m.bus.set_pms_write_bypass(true);
            probe_all(&mut m.bus, &regs)
        }
        Arch::Xtensa => {
            // Build the real per-chip peripheral set, the same way the runtime
            // does, so coverage reflects the actual model — not the vestigial
            // chip-yaml peripheral list that from_config seeded above (these
            // system builders clear and repopulate the bus). esp32s3 previously
            // fell through to the generic `configure_xtensa`, which registers no
            // peripherals, so its coverage was measured against the yaml stub
            // only. Mirrors cli::coverage::build_matrix.
            let cpu = match chip.name.as_str() {
                "esp32" => system::xtensa::configure_xtensa_esp32(&mut bus),
                "esp32s3" => {
                    system::xtensa::configure_xtensa_esp32s3(
                        &mut bus,
                        &system::xtensa::Esp32s3Opts::default(),
                    )
                    .cpu
                }
                _ => system::xtensa::configure_xtensa(&mut bus),
            };
            let mut m = Machine::new(cpu, bus);
            m.bus.set_clock_gating_bypass(true);
            probe_all(&mut m.bus, &regs)
        }
        // No probe path exists for these. Returning (0,0,0) here was the fourth
        // fail-open: a measured zero ratchets against a baseline zero forever, so
        // the chip would sit in the table looking covered and gating nothing.
        // AVR P0 has no SVD/register-map probe yet — such a chip belongs out of
        // CHIPS (estate-only, tracked by chip_conformance) until one lands.
        arch @ (Arch::Avr | Arch::Unknown) => panic!(
            "{name}: arch {arch:?} has no register probe path, so its coverage would \
             measure 0 and ratchet against 0 — a gate that can never fail. Remove it \
             from CHIPS (and from docs/coverage/register-modeling.json) until a probe \
             exists, or add one here."
        ),
    };
    (total, mapped, reset_ok, modeled)
}

/// CI gate: per-chip register-modeling coverage may never regress.
///
/// The baseline lives at `docs/coverage/register-modeling.json`. Each chip's
/// `modeled` count must stay >= its baseline. Regenerate the baseline (after an
/// intentional model change) with:
/// ```text
/// UPDATE_COVERAGE_BASELINE=1 cargo test -p labwired-core --test register_coverage -- --nocapture
/// ```
#[test]
fn register_coverage_ratchet() {
    // Chip yamls reference peripheral descriptors by paths relative to
    // configs/chips/ (resolved against CWD). root()/SVD reads stay absolute.
    let baseline_path = root("docs/coverage/register-modeling.json");
    std::env::set_current_dir(root("configs/chips")).expect("cd configs/chips");

    let mut current = serde_json::Map::new();
    println!(
        "\nregister-modeling coverage vs vendor SVD\n{:<11} {:>6} {:>8} {:>9} {:>9}",
        "chip", "total", "mapped", "reset_ok", "modeled"
    );
    println!("{}", "-".repeat(50));
    for &(name, yaml, svd) in CHIPS {
        // `svd: None` is the ONLY legitimate "pending": a deliberate, committed
        // statement that no public vendor SVD exists for this part. An SVD that
        // is present but unreadable/unparseable now panics inside measure_chip
        // instead of demoting the chip to "pending" and skipping its gate.
        let Some(svd) = svd else {
            println!(
                "{name:<11} {:>6}   (SVD pending — no public vendor SVD yet)",
                "-"
            );
            current.insert(name.to_string(), serde_json::json!({"svd_pending": true}));
            continue;
        };
        let (total, mapped, reset_ok, modeled) = measure_chip(name, yaml, svd);
        let pct = if total > 0 {
            modeled as f64 * 100.0 / total as f64
        } else {
            0.0
        };
        println!("{name:<11} {total:>6} {mapped:>8} {reset_ok:>9} {modeled:>5} ({pct:>4.1}%)");
        current.insert(
            name.to_string(),
            serde_json::json!({"total": total, "modeled": modeled}),
        );
    }
    println!();

    if std::env::var("UPDATE_COVERAGE_BASELINE").is_ok() {
        std::fs::write(
            &baseline_path,
            serde_json::to_string_pretty(&current).unwrap() + "\n",
        )
        .expect("write baseline");
        println!("updated baseline: {}", baseline_path.display());
        return;
    }

    let baseline: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path)
            .expect("baseline missing — run with UPDATE_COVERAGE_BASELINE=1"),
    )
    .expect("parse baseline");

    // The measured chip set and the baseline key set must be IDENTICAL.
    //
    // Without this, `baseline[name]["modeled"].as_u64().unwrap_or(0)` silently
    // ratcheted a chip missing from the baseline against zero: rename a chip, or
    // add one, and it could never fail. The mirror hole let a chip disappear from
    // CHIPS entirely while its baseline entry sat there unread. Adding or removing
    // a chip is now a deliberate act that must touch both files in one commit.
    let baseline_obj = baseline
        .as_object()
        .expect("baseline is a JSON object of chip -> {modeled,total}");
    let measured_names: std::collections::BTreeSet<&str> =
        current.keys().map(|s| s.as_str()).collect();
    let baseline_names: std::collections::BTreeSet<&str> =
        baseline_obj.keys().map(|s| s.as_str()).collect();
    let missing_from_baseline: Vec<&&str> = measured_names.difference(&baseline_names).collect();
    let missing_from_chips: Vec<&&str> = baseline_names.difference(&measured_names).collect();
    assert!(
        missing_from_baseline.is_empty() && missing_from_chips.is_empty(),
        "chip set and baseline key set disagree — a chip outside the baseline ratchets \
         against nothing:\n  measured but absent from {}: {missing_from_baseline:?}\n  \
         in the baseline but not measured: {missing_from_chips:?}\n  \
         (intentional? re-baseline with UPDATE_COVERAGE_BASELINE=1 in the same commit)",
        baseline_path.display()
    );

    let mut regressions = Vec::new();
    for (name, cur) in &current {
        let base = &baseline[name];
        // A chip that HAD a measurement may not fall back to "pending". That
        // transition is exactly what a deleted/corrupted SVD used to look like.
        let Some(cur_modeled) = cur["modeled"].as_u64() else {
            if base["modeled"].as_u64().is_some() {
                regressions.push(format!(
                    "{name}: was measured in the baseline but is now SVD-pending — a \
                     measured chip may not lose its gate"
                ));
            }
            continue;
        };
        // No `unwrap_or(0)`: the key is guaranteed present by the set check
        // above, and a present-but-shapeless entry is a corrupt baseline, not a
        // free pass.
        let Some(base_modeled) = base["modeled"].as_u64() else {
            // Baseline says pending, we now measure it: that is new coverage, not
            // a regression — but only if the baseline actually said so.
            assert!(
                base["svd_pending"].as_bool() == Some(true),
                "{name}: baseline entry has neither `modeled` nor `svd_pending`: {base}"
            );
            continue;
        };
        if cur_modeled < base_modeled {
            regressions.push(format!(
                "{name}: modeled regressed {base_modeled} -> {cur_modeled}"
            ));
        }
    }
    assert!(
        regressions.is_empty(),
        "register-modeling coverage regressed:\n  {}",
        regressions.join("\n  ")
    );
}
