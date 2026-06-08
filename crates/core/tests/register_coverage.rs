// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Register-modeling coverage vs the vendor SVD.
//!
//! For each chip with an in-tree SVD, this enumerates every register the
//! datasheet defines (via `svd-ingestor`) and probes the simulator's bus to
//! measure how many are actually modeled. It is a **measurement**, not a gate
//! (`#[ignore]`); run it with:
//!
//! ```text
//! cargo test -p labwired-core --test register_coverage -- --ignored --nocapture
//! ```
//!
//! Per register we record three signals from the live bus:
//!   * `mapped`     — a read succeeds (the address lands in a modeled peripheral)
//!   * `reset_ok`   — the read value equals the SVD reset value
//!   * `responsive` — writing 0xFFFF_FFFF then 0 yields different read-backs
//!                    (the register stores state — definitive proof of modeling)
//!
//! Headline `modeled` is the conservative union: `responsive || (reset_ok &&
//! reset != 0)`. It under-counts write-only and read-only-reset-0 registers
//! that are modeled but indistinguishable from an unhandled-offset default, so
//! treat it as a lower bound; `mapped` is the upper bound.

use labwired_config::{Arch, ChipDescriptor};
use labwired_core::bus::SystemBus;
use labwired_core::{system, Machine};
use std::path::PathBuf;

/// (chip name, chip yaml, in-tree SVD).
const CHIPS: &[(&str, &str, &str)] = &[
    (
        "stm32f401",
        "configs/chips/stm32f401.yaml",
        "tests/fixtures/real_world/stm32f401.svd",
    ),
    (
        "esp32c3",
        "configs/chips/esp32c3.yaml",
        "tests/fixtures/real_world/esp32c3.svd",
    ),
    (
        "nrf52832",
        "configs/chips/nrf52832.yaml",
        "tests/fixtures/real_world/nrf52832.svd",
    ),
    (
        "rp2040",
        "configs/chips/rp2040.yaml",
        "tests/fixtures/real_world/rp2040.svd",
    ),
    (
        "esp32s3",
        "configs/chips/esp32s3.yaml",
        "tests/fixtures/svd/esp32s3.svd",
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
        walk_deleted: false,
        schema_version: "1.0".to_string(),
        name: "coverage".to_string(),
        chip: path.to_string(),
        external_devices: vec![],
        board_io: vec![],
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
fn svd_registers(svd_path: &str) -> Vec<(u64, u32)> {
    let xml = std::fs::read_to_string(root(svd_path)).expect("read SVD");
    let device = svd_parser::parse(&xml).expect("parse SVD");
    let mut out = Vec::new();
    for peripheral in &device.peripherals {
        let base = peripheral.base_address;
        let desc = match svd_ingestor::process_peripheral(&device, peripheral) {
            Ok(d) => d,
            Err(_) => continue,
        };
        for reg in &desc.registers {
            out.push((base + reg.address_offset, reg.reset_value));
        }
    }
    out
}

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

/// One chip's measured coverage: (total SVD registers, mapped, reset_ok, modeled).
fn measure_chip(yaml: &str, svd: &str) -> (usize, usize, usize, usize) {
    let regs = svd_registers(svd);
    let total = regs.len();
    let chip = ChipDescriptor::from_file(&root(yaml)).expect("chip yaml");
    let mut bus = SystemBus::from_config(&chip, &dummy_manifest(yaml)).expect("bus");
    let (mapped, reset_ok, modeled) = match chip.arch {
        Arch::Arm => {
            let (cpu, _nvic) = system::cortex_m::configure_cortex_m(&mut bus);
            let mut m = Machine::new(cpu, bus);
            probe_all(&mut m.bus, &regs)
        }
        Arch::RiscV => {
            let cpu = system::riscv::configure_riscv(&mut bus);
            let mut m = Machine::new(cpu, bus);
            probe_all(&mut m.bus, &regs)
        }
        Arch::Xtensa => {
            let cpu = if chip.name == "esp32" {
                system::xtensa::configure_xtensa_esp32(&mut bus)
            } else {
                system::xtensa::configure_xtensa(&mut bus)
            };
            let mut m = Machine::new(cpu, bus);
            probe_all(&mut m.bus, &regs)
        }
        Arch::Unknown => (0, 0, 0),
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
        let (total, mapped, reset_ok, modeled) = measure_chip(yaml, svd);
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

    let mut regressions = Vec::new();
    for (name, cur) in &current {
        let cur_modeled = cur["modeled"].as_u64().unwrap();
        let base_modeled = baseline[name]["modeled"].as_u64().unwrap_or(0);
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
