// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The contract, stated without reference to how the engine is built:
//!
//! > A display the author placed answers ONE call, by the name the author gave
//! > it, and tells the caller enough to render it sight-unseen.
//!
//! Not "an SSD1306 answers `get_ssd1306_framebuffer` if a `board_io` binding
//! names it and the caller's controller list happens to include this chip's SPI
//! block". Every clause of that sentence was a way for a working, painting panel
//! to render blank, and each one had to be discovered by a customer.
//!
//! # Where the fixtures come from
//!
//! Not from the accessor's own match arms — a list built that way agrees with
//! the code by construction and proves nothing. They are the shipped
//! `system.yaml` files, scanned for the shape that broke: a display declared
//! under `external_devices:` that NO `board_io:` entry names. Every accessor
//! this replaces began by looking that display up in `board_io`, so on these
//! rigs the browser had no placement to query and painted nothing — whatever the
//! model, whatever the chip.
//!
//! Deliberately not one model and not one transport, so the fix cannot be
//! shaped like any single panel: an SPI TFT, an I²C OLED, and a GPIO-bit-banged
//! 7-segment module that has no controller to hang off at all.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::{artifact_format as fmt, InspectOpts};
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Shipped rigs that place a display under `external_devices:` with no
/// `board_io:` entry naming it, paired with the id the author gave it.
///
/// Found by scanning every `system.yaml` in the tree for that shape, not by
/// reading any accessor.
const BOARD_IO_LESS_DISPLAYS: &[(&str, &str, &str)] = &[
    // Ryan's bay-occupancy rig: an ILI9341 on the classic-ESP32 SPI3.
    ("examples/esp32-bay-occupancy/system.yaml", "tft", "ili9341"),
    // The same shape with a different model on a different transport and a
    // different chip — so a fix that only works for the TFT fails here.
    (
        "examples/esp32c3-leo-airquality/system.yaml",
        "oled",
        "oled-ssd1306",
    ),
    // And one with no controller at all: bit-banged on GPIO pins, held in a
    // bus-resident collection rather than inside a peripheral.
    ("examples/tm1637-7seg-lab/system.yaml", "seg", "tm1637-7seg"),
];

/// Assemble the rig with no firmware, by the same route the browser assembles
/// it for this chip family. Placement is a property of the manifest; nothing has
/// to run for a display to be wired, and a test that booted firmware first would
/// be measuring the boot.
fn bus_from_example(rel_yaml: &str) -> SystemBus {
    let yaml = repo(rel_yaml);
    let manifest = SystemManifest::from_file(&yaml).expect("load system.yaml");
    let chip_path = yaml.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut bus = match chip.arch {
        // The classic ESP32 builds its peripherals from the CPU configuration
        // rather than from the chip's peripheral list, so its external devices
        // are attached afterwards — see `WasmSimulator::new_from_config`.
        labwired_config::Arch::Xtensa => {
            let mut bus = SystemBus::new();
            let _ = labwired_core::system::xtensa::configure_xtensa_esp32(&mut bus);
            labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
                .expect("attach external devices");
            bus
        }
        labwired_config::Arch::RiscV => {
            let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
            let _ = labwired_core::system::riscv::configure_riscv(&mut bus);
            bus
        }
        _ => {
            let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
            let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
            bus
        }
    };
    bus.refresh_peripheral_index();
    bus
}

/// The one door answers for every shipped display, whatever bound it.
#[test]
fn every_placed_display_answers_the_one_door() {
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };

    for (lab, id, device_type) in BOARD_IO_LESS_DISPLAYS {
        let bus = bus_from_example(lab);
        let artifact = bus.display_artifact(id, &opts).unwrap_or_else(|| {
            panic!(
                "{lab}: the author placed a {device_type} called '{id}', and the engine built \
                 it — but the one door returns nothing, so nothing can render it"
            )
        });

        // Geometry and packing must come back as DATA. A caller that has never
        // heard of this model must still be able to paint it, which is exactly
        // what a per-accessor doc contract ("153,600 bytes = 240x320x2,
        // big-endian RGB565") cannot give it.
        assert!(
            artifact
                .meta
                .get("format")
                .and_then(|v| v.as_str())
                .is_some(),
            "{lab}: '{id}' must say how its bytes are packed"
        );
        assert!(
            !artifact.id.is_empty(),
            "{lab}: '{id}' must come back stamped with the id it was addressed by"
        );
    }
}

/// The legacy-spelling regression, restated as behaviour.
///
/// core#759 fixed this at the level of a string comparison: the two SSD1680
/// accessors each inlined two of `TYPE_ALIASES`' three rows, dropped
/// `gxepd2_290_c90c`, and a lab authored with that spelling attached a real
/// panel, drove it, and rendered dark. The fix routed both through
/// `canonical_device_type`.
///
/// The one door removes the comparison rather than correcting it: resolution
/// joins on the id the author gave the device and on where it was found, so no
/// spelling of `device_type` is ever matched and no alias table is ever
/// consulted. That is a stronger guarantee than canonicalising — there is no
/// list left to be missing a row from — but "stronger by construction" is an
/// argument, and #759 was closed with evidence. So this asserts the OUTCOME the
/// fix existed for, on the exact spelling that was dropped: a panel declared as
/// `gxepd2_290_c90c` renders.
///
/// The two other rows are checked alongside it, so a future alias that resolves
/// in the engine but not in the browser fails here rather than in a lab.
#[test]
fn a_panel_declared_by_any_legacy_spelling_still_renders() {
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    // Every spelling `registry::lookup` accepts for this panel, canonical first.
    for spelling in [
        "ssd1680_tricolor_290",
        "epd-2in9-tricolor",
        "gxepd2_290_c90c",
    ] {
        let yaml = format!(
            r#"
name: "alias-fixture"
chip: "../chips/stm32f103.yaml"
external_devices:
  - id: "epaper"
    type: "{spelling}"
    connection: "spi1"
    config:
      cs_pin: "PA4"
"#
        );
        let manifest: SystemManifest =
            serde_yaml::from_str(&yaml).expect("parse the synthesized manifest");
        let chip_path = repo("configs/chips/stm32f103.yaml");
        let chip = ChipDescriptor::from_file(&chip_path).expect("chip");
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap_or_else(|e| {
            panic!("{spelling}: the engine refused to build a panel it accepts: {e}")
        });
        let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        bus.refresh_peripheral_index();

        let artifact = bus.display_artifact("epaper", &opts).unwrap_or_else(|| {
            panic!(
                "a panel declared as `{spelling}` attaches and is driven, but the one door \
                 returns nothing for it — this is exactly the dark-panel regression core#759 \
                 closed, reopened"
            )
        });
        assert_eq!(
            artifact.meta.get("format").and_then(|v| v.as_str()),
            Some(fmt::EPAPER_TRICOLOR_PLANES),
            "`{spelling}` must resolve to the same panel as its canonical spelling"
        );
        assert_eq!(
            artifact.bytes.as_ref().map(Vec::len),
            Some(9472),
            "`{spelling}` must hand back both planes, not a truncated buffer"
        );
    }
}

/// The ratchet: **every** display this engine can see, in every rig that ships,
/// answers the one door.
///
/// The enumeration is the FILESYSTEM — every `system.yaml` under `examples/`
/// and `configs/systems/` — and the set of displays inside each one is whatever
/// reports a display-kind artifact through the walk. Nothing here names a model,
/// a controller, a transport, or a device_type. That matters: a list built by
/// reading the accessor's own arms agrees with the accessor by construction and
/// would pass with the door deleted.
///
/// So the failure this makes impossible is the one that keeps happening — a new
/// display model lands, ships in a lab, and nobody remembers the second place it
/// had to be registered. There is no second place left to forget, and if one
/// grows back this test is what notices.
///
/// What it does NOT catch, stated so nobody reads more into a green run than is
/// there: a model that reports no evidence AT ALL is invisible to the walk and
/// therefore invisible here too. That direction — "this part is a screen, so its
/// model owes us pixels" — belongs to the part-simulation ratchet in the
/// superproject catalog, which is where the claim "this part is a display"
/// actually lives.
#[test]
fn every_display_in_every_shipped_rig_answers_the_one_door() {
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let mut rigs_with_a_display = 0;
    let mut displays_checked = 0;

    for yaml in shipped_system_manifests() {
        // A rig this test harness cannot assemble (a chip family with a
        // bespoke boot path, a fixture that needs blobs) is skipped rather
        // than failed — it is not evidence about the door either way.
        let Some(bus) = try_bus_from_example(&yaml) else {
            continue;
        };
        let mut found_here = false;

        for device in bus.inspect_devices(None, &opts) {
            for artifact in device
                .artifacts
                .iter()
                .filter(|a| labwired_core::inspect::is_display_artifact(a))
            {
                found_here = true;
                displays_checked += 1;
                let through_the_door =
                    bus.display_artifact(&device.id, &opts).unwrap_or_else(|| {
                        panic!(
                            "{yaml}: the walk can see a '{}' display for '{}', but the one door \
                             returns nothing for that id — the browser would paint it blank",
                            artifact.kind, device.id
                        )
                    });
                assert_eq!(
                    &through_the_door.bytes, &artifact.bytes,
                    "{yaml}: '{}' — the door and the walk must return the SAME pixels",
                    device.id
                );
                assert_eq!(
                    &through_the_door.meta, &artifact.meta,
                    "{yaml}: '{}' — and the same geometry and packing",
                    device.id
                );
            }
        }
        if found_here {
            rigs_with_a_display += 1;
        }
    }

    println!(
        "one door served {displays_checked} displays across {rigs_with_a_display} shipped rigs"
    );
    // Without this the test passes just as happily if the scan finds nothing,
    // which is the shape of every gate that was green while broken.
    assert!(
        rigs_with_a_display >= 8 && displays_checked >= 8,
        "the scan found only {displays_checked} displays across {rigs_with_a_display} rigs; \
         the repo ships far more than that, so the enumeration has silently stopped working"
    );
}

/// Every `system.yaml` the repo ships, found by walking the tree — not by
/// reading a list that some other file also has to be told about.
fn shipped_system_manifests() -> Vec<String> {
    let mut out = Vec::new();
    for dir in ["examples", "configs/systems"] {
        collect_manifests(&repo(dir), dir, &mut out);
    }
    out.sort();
    out
}

fn collect_manifests(dir: &std::path::Path, rel: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            collect_manifests(&path, &format!("{rel}/{name}"), out);
        } else if name.ends_with(".yaml") || name.ends_with(".yml") {
            out.push(format!("{rel}/{name}"));
        }
    }
}

/// [`bus_from_example`] for a file that may not be a system manifest at all, or
/// may need a boot path this harness does not have.
fn try_bus_from_example(rel_yaml: &str) -> Option<SystemBus> {
    std::panic::catch_unwind(|| bus_from_example(rel_yaml)).ok()
}
