// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The `simctl` ABI has ONE source and no copies.
//!
//! The register offsets live in `peripherals::simctl`. The firmware header is
//! generated from them by `tools/gen_simctl_header.py`, and the board manifest
//! is the only other place an address appears — the base, which the generator
//! reads back out of that manifest.
//!
//! An earlier version of this file parsed the hand-written header and compared
//! it to the model. That policed a drift class instead of removing it; these
//! tests assert the generator's output is what is on disk, so the header cannot
//! be edited into disagreement in the first place.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::simctl::{self, SimCtl};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const SELFTEST_MANIFEST: &str = "configs/systems/pico-selftest.yaml";

fn read_selftest_manifest() -> SystemManifest {
    let path = root(SELFTEST_MANIFEST);
    let mut manifest = SystemManifest::from_file(&path)
        .unwrap_or_else(|e| panic!("cannot load {SELFTEST_MANIFEST}: {e}"));
    let anchored = path.parent().expect("manifest parent").join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    manifest
}

fn declared_simctl() -> labwired_config::PeripheralConfig {
    read_selftest_manifest()
        .peripherals
        .into_iter()
        .find(|p| p.r#type == "simctl")
        .unwrap_or_else(|| panic!("{SELFTEST_MANIFEST} no longer declares a simctl peripheral"))
}

#[test]
fn the_firmware_header_is_what_the_generator_produces() {
    // The whole anti-drift mechanism, in one assertion: if anyone hand-edits
    // the header, or changes a `pub const` without regenerating, this fails.
    let output = Command::new("python3")
        .arg(root("tools/gen_simctl_header.py"))
        .arg("--check")
        .output()
        .expect("run the header generator");

    assert!(
        output.status.success(),
        "examples/common/labwired_simctl.h is out of date with the model.\n\
         Run: python3 tools/gen_simctl_header.py\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_board_declares_exactly_the_devices_window() {
    // A declared window smaller than the register map silently drops the high
    // registers; a larger one claims space the device does not answer for.
    let declared = declared_simctl();
    let size = declared
        .size
        .as_deref()
        .map(|s| labwired_config::parse_size(s).expect("parse the declared size"))
        .unwrap_or_else(|| panic!("{SELFTEST_MANIFEST} should declare an explicit size"));

    assert_eq!(size, simctl::WINDOW);
}

#[test]
fn the_shipped_board_actually_builds_the_device() {
    // Reads the real YAML off disk, not a literal built in the test: this is
    // the artifact a user gets.
    let manifest = read_selftest_manifest();
    let chip = ChipDescriptor::from_file(&manifest.chip).expect("load the manifest's chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build the pico-selftest bus");
    let base = declared_simctl().base_address;

    let entry = bus
        .peripherals
        .iter()
        .find(|p| p.base == base)
        .unwrap_or_else(|| panic!("no peripheral landed at {base:#x}"));

    assert!(
        entry
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<SimCtl>())
            .is_some(),
        "the peripheral at the declared base must be a SimCtl, not a stub"
    );
}

#[test]
fn the_device_does_not_collide_with_real_rp2040_silicon() {
    // The window is deliberately outside the chip's real peripheral space, so
    // it can never be mistaken for hardware. If a future RP2040 descriptor
    // grows a peripheral here, this fails before anyone debugs a phantom.
    let manifest = read_selftest_manifest();
    let chip = ChipDescriptor::from_file(&manifest.chip).expect("load the manifest's chip");
    let base = declared_simctl().base_address;

    for p in &chip.peripherals {
        assert!(
            p.base_address + simctl::WINDOW <= base || p.base_address >= base + simctl::WINDOW,
            "silicon peripheral `{}` at {:#x} overlaps the simctl window at {base:#x}",
            p.id,
            p.base_address
        );
    }
    assert!(
        base < chip.flash.base || base >= chip.flash.base + 0x0200_0000,
        "the simctl window overlaps the chip's flash region"
    );
    assert!(
        base < chip.ram.base || base >= chip.ram.base + 0x0010_0000,
        "the simctl window overlaps the chip's RAM region"
    );
}

#[test]
fn the_plain_pico_board_is_unchanged() {
    // The opt-in claim as an assertion: this feature must not put a device on
    // the board that existed before it.
    let path = root("configs/systems/pico.yaml");
    let mut manifest = SystemManifest::from_file(&path).expect("load pico.yaml");
    let anchored = path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored.to_string_lossy().into_owned();
    let chip = ChipDescriptor::from_file(&manifest.chip).expect("load rp2040 chip");
    let bus = SystemBus::from_config(&chip, &manifest).expect("build the plain pico bus");

    assert!(
        bus.peripherals.iter().all(|p| p
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<SimCtl>())
            .is_none()),
        "the plain `pico` board must not carry a simctl device"
    );
}
