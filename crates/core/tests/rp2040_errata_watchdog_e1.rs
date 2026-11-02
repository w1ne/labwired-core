// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Executable model check for RP2040 erratum E1.
//!
//! Datasheet Appendix B, RP2040-E1: *"There is a logic error which means the
//! counter is decremented twice per tick, instead of once per tick."* The chip
//! descriptor is the corpus entry point for that erratum, and this test is what
//! makes it executable rather than a footnote:
//!
//! 1. the descriptor really declares `RP2040-E1` on the watchdog instance, and
//!    the `model_check` it names really exists in the tree;
//! 2. the watchdog, driven over its real MMIO registers, really counts down in
//!    TWOS — i.e. the model reproduces the vendor defect and not the
//!    datasheet's idealised "once per tick".
//!
//! If a future edit makes the model count down once per tick, step 2 fails and
//! the `proven` claim for this erratum has to be withdrawn. That is the point:
//! the corpus row and the behaviour cannot drift apart silently.
//!
//! Deliberately NOT behind the `event-scheduler` feature. The forced peripheral
//! tick below is the same countdown path the scheduler drains, and an
//! unconditional test is the stronger claim — a feature-gated file compiles to
//! nothing and reports `0 passed` as a pass.

mod common;

use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

const WATCHDOG_BASE: u64 = 0x4005_8000;
const CTRL: u64 = WATCHDOG_BASE;
const LOAD: u64 = WATCHDOG_BASE + 0x04;
const TICK: u64 = WATCHDOG_BASE + 0x2C;
const CTRL_ENABLE: u32 = 1 << 30;
const TICK_ENABLE: u32 = 1 << 9;
const MODEL_CHECK: &str =
    "crates/core/src/peripherals/rp2040/watchdog.rs::tests::counter_decrements_twice_per_tick";

fn rp2040_bus() -> SystemBus {
    let system_path = root("configs/systems/rp2040-pico.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load rp2040-pico");
    let chip_path = system_path
        .parent()
        .expect("system manifest parent")
        .join(&manifest.chip);
    manifest.chip = chip_path.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(chip_path).expect("load rp2040 descriptor");
    SystemBus::from_config(&chip, &manifest).expect("build rp2040 bus")
}

/// The descriptor declares the erratum, and the test it names exists.
#[test]
fn rp2040_e1_is_declared_on_the_watchdog_descriptor() {
    let chip = ChipDescriptor::from_file(root("configs/chips/rp2040.yaml"))
        .expect("load real rp2040 descriptor");
    let watchdog = chip
        .peripherals
        .iter()
        .find(|peripheral| peripheral.id == "watchdog")
        .expect("rp2040 descriptor declares watchdog");
    let errata = watchdog
        .config
        .get("errata")
        .and_then(serde_yaml::Value::as_sequence)
        .expect("watchdog errata block survives descriptor parsing");
    let e1 = errata
        .iter()
        .find(|entry| entry.get("id").and_then(serde_yaml::Value::as_str) == Some("RP2040-E1"))
        .expect("watchdog declares RP2040-E1");
    assert_eq!(
        e1.get("model_check").and_then(serde_yaml::Value::as_str),
        Some(MODEL_CHECK),
        "descriptor must point at this stable model check"
    );

    let (source_path, test_path) = MODEL_CHECK.split_once("::").expect("model check path");
    let source = std::fs::read_to_string(root(source_path)).expect("model-check source exists");
    let test_name = test_path.rsplit("::").next().expect("test name");
    assert!(
        source.contains("mod tests") && source.contains(&format!("fn {test_name}")),
        "model-check test does not exist at {MODEL_CHECK}"
    );
}

/// The model reproduces the defect: 100 loaded counts, ten ticks, 80 left.
#[test]
fn the_watchdog_counts_down_twice_per_tick() {
    let mut bus = rp2040_bus();
    // RP2040 datasheet §4.7: TICK @ +0x2c, LOAD @ +0x04, CTRL @ +0x00.
    // A divide ratio of one produces one watchdog tick per peripheral tick.
    bus.write_u32(TICK, 1 | TICK_ENABLE).expect("program TICK");
    bus.write_u32(LOAD, 100).expect("load watchdog counter");
    bus.write_u32(CTRL, CTRL_ENABLE).expect("enable watchdog");

    for _ in 0..10 {
        bus.tick_peripherals_fully_forced();
    }

    let remaining = bus.read_u32(CTRL).expect("read CTRL") & 0x00FF_FFFF;
    assert_eq!(
        remaining, 80,
        "RP2040-E1 model check: after ten ticks, 100 loaded counts must decrement by two each tick"
    );
}
