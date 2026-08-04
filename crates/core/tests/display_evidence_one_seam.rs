// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The contract, stated without reference to how the engine is built:
//!
//! > A panel that `inspect` can show pixels for is a panel the renderer can
//! > show pixels for, and they are the SAME pixels.
//!
//! This is the property the browser lost for a year. `crates/wasm` reached its
//! panels through a hand-written matrix of controller downcasts; `inspect`
//! reached them through the device walk. Two systems, one sighted, one blind —
//! and which one was blind depended on which arm somebody had remembered to add.
//! The SH1107 painted a full console in the browser while `inspect` reported no
//! artifact at all.
//!
//! The two surfaces are keyed DIFFERENTLY on purpose, and that is what makes
//! this test worth writing rather than a mirror of the implementation:
//!
//! * [`SystemBus::inspect_devices`] joins on the manifest — "the device the
//!   author called `epaper`".
//! * [`SystemBus::device_artifact_at`] joins on placement — "the tri-color
//!   e-paper on `spi1`", which is all a `board_io` binding actually knows.
//!
//! Different keys, and the answer must be byte-identical. Nothing below
//! downcasts to a panel type, names a controller, or restates the lookup: the
//! fixtures are the `system.yaml` files that ship in `examples/`, and the
//! expectation is read out of the OTHER surface.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::{artifact_format as fmt, Artifact, InspectOpts};
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Assemble the rig with no firmware. Placement is a property of the manifest;
/// nothing has to run for a panel to be wired, and a test that booted firmware
/// first would be measuring the boot.
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

/// Every artifact `inspect` reports for a device that has a `format`, with the
/// placement it was found at — i.e. everything the renderer would need to ask
/// the other question.
struct Reported {
    id: String,
    bus: String,
    address: Option<u8>,
    artifact: Artifact,
}

fn reported_displays(bus: &SystemBus, opts: &InspectOpts) -> Vec<Reported> {
    bus.inspect_devices(None, opts)
        .into_iter()
        .flat_map(|d| {
            let bus = d.attachment.bus.clone();
            let address = d.attachment.address;
            let id = d.id.clone();
            d.artifacts.into_iter().filter_map(move |a| {
                let bus = bus.clone()?;
                a.meta.get("format")?.as_str()?;
                Some(Reported {
                    id: id.clone(),
                    bus,
                    address,
                    artifact: a,
                })
            })
        })
        .collect()
}

/// Every shipped rig that places a display, across the transports and
/// controller families the deleted wasm accessors enumerated by hand: a
/// generic-I²C OLED, an ESP32-C3 I²C OLED, an ESP32-S3 I²C SH1107, a
/// generic-SPI TFT, a generic-SPI e-paper, and an ESP32 SPI e-paper.
const DISPLAY_LABS: &[&str] = &[
    "examples/ssd1306-hello-lab/system.yaml",
    "examples/ssd1306-128x32-lab/system.yaml",
    "examples/ili9341-tft-lab/system.yaml",
    "examples/epaper-tricolor-lab/system.yaml",
    "configs/systems/esp32c3-oled-demo.yaml",
    "configs/systems/esp32c3-oled-128x32-workshop.yaml",
    "configs/systems/esp32s3-oled-demo.yaml",
    "configs/systems/esp32c3-epaper-workshop.yaml",
    "configs/systems/esp32-wroom-epaper.yaml",
    "configs/systems/nucleo-f103rb-epaper.yaml",
];

/// The whole contract, for every shipped rig that has a panel.
///
/// Asking by placement must return, byte for byte, what asking by manifest id
/// returned. A controller the walk cannot reach fails BOTH halves and so shows
/// up as "this lab reported no display at all"; a controller the placement
/// query mishandles fails only the second half.
#[test]
fn placement_query_returns_what_inspect_reports_for_every_shipped_panel() {
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let mut panels_checked = 0;

    for lab in DISPLAY_LABS {
        let bus = bus_from_example(lab);
        let reported = reported_displays(&bus, &opts);
        assert!(
            !reported.is_empty(),
            "{lab} places a display but inspect reports no artifact for any device — \
             the walk cannot reach this lab's panel, so nothing can render it"
        );

        for r in &reported {
            let format = r.artifact.meta["format"].as_str().expect("format");
            // Ask the OTHER question, with only what a `board_io` binding
            // knows: which controller, and (for an addressed transport) which
            // address.
            let by_placement = bus
                .device_artifact_at(&r.bus, r.address, &[format], &r.id, &opts)
                .unwrap_or_else(|| {
                    panic!(
                        "{lab}: inspect reports a '{format}' artifact for '{}' on '{}' \
                         (address {:?}), but the placement query finds nothing there",
                        r.id, r.bus, r.address
                    )
                });

            assert_eq!(
                by_placement.kind, r.artifact.kind,
                "{lab}: '{}' — same device, so same artifact kind",
                r.id
            );
            assert_eq!(
                by_placement.id, r.artifact.id,
                "{lab}: '{}' — the artifact is stamped with the id it was addressed by",
                r.id
            );
            assert_eq!(
                by_placement.meta, r.artifact.meta,
                "{lab}: '{}' — the renderer and inspect must not disagree about the \
                 panel's dimensions, format, or ink",
                r.id
            );
            assert_eq!(
                by_placement.bytes, r.artifact.bytes,
                "{lab}: '{}' — the renderer and inspect must paint the SAME pixels",
                r.id
            );
            panels_checked += 1;
        }
    }

    assert!(
        panels_checked >= DISPLAY_LABS.len(),
        "every listed lab must contribute at least one panel; only {panels_checked} were checked, \
         which means a fixture stopped placing a display and this test went quiet"
    );
}

/// The negative direction, which is what makes the positive one mean something.
///
/// A query is only as good as its ability to say no. If `device_artifact_at`
/// answered regardless of the placement it was handed, the assertions above
/// would pass with the lookup deleted.
#[test]
fn placement_query_refuses_the_wrong_placement() {
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let bus = bus_from_example("examples/ssd1306-hello-lab/system.yaml");
    let reported = reported_displays(&bus, &opts);
    let panel = reported.first().expect("the lab places an OLED");
    let format = panel.artifact.meta["format"].as_str().expect("format");

    assert!(
        bus.device_artifact_at(&panel.bus, panel.address, &[format], &panel.id, &opts)
            .is_some(),
        "control: the real placement answers"
    );
    assert!(
        bus.device_artifact_at("i2c99", panel.address, &[format], &panel.id, &opts)
            .is_none(),
        "a controller the panel is not on must not answer for it"
    );
    assert!(
        bus.device_artifact_at(&panel.bus, Some(0x77), &[format], &panel.id, &opts)
            .is_none(),
        "an address the panel does not answer to must not answer for it"
    );
    assert!(
        bus.device_artifact_at(
            &panel.bus,
            panel.address,
            &[fmt::RGB565_BE],
            &panel.id,
            &opts
        )
        .is_none(),
        "a panel must not be handed to a caller asking for a different format — \
         that is what stops an OLED being painted as a TFT framebuffer"
    );
}

/// Summary mode must still identify the panel and still carry the cheap `meta`
/// the UI polls, because that is the whole reason the e-paper refresh counter
/// does not have to drag 9472 bytes across the wasm boundary sixty times a
/// second.
#[test]
fn summary_mode_keeps_the_metadata_and_drops_only_the_payload() {
    let bus = bus_from_example("examples/epaper-tricolor-lab/system.yaml");
    let full = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let summary = InspectOpts {
        include_bytes: false,
        peripheral: None,
    };
    let reported = reported_displays(&bus, &full);
    let panel = reported.first().expect("the lab places an e-paper panel");

    let lean = bus
        .device_artifact_at(
            &panel.bus,
            panel.address,
            &[fmt::EPAPER_TRICOLOR_PLANES],
            &panel.id,
            &summary,
        )
        .expect("summary mode still finds the panel");

    assert!(lean.bytes.is_none(), "summary mode drops the payload");
    assert_eq!(
        lean.meta, panel.artifact.meta,
        "summary mode changes nothing but the payload — the UI's refresh_generation \
         poll reads the same number the full read would report"
    );
    assert!(
        lean.meta.get("refresh_generation").is_some(),
        "the counter the renderer polls before re-fetching pixels must survive summary mode"
    );
}
