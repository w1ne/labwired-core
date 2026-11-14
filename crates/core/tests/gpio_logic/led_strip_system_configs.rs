// SPDX-License-Identifier: MIT

//! Every shipped system that declares an addressable LED strip must build, on
//! whatever GPIO family its chip uses. The strip's data pin is parsed with the
//! shared pad resolver, so a family it cannot name (SAMD `PA15`) used to break
//! the whole board silently.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::kit::AttachCtx;
use std::path::PathBuf;

const STRIP_TYPES: [&str; 3] = ["neopixel", "ws2812", "apa102"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn build(system: &std::path::Path) -> (ChipDescriptor, SystemManifest) {
    let manifest = SystemManifest::from_file(system).unwrap();
    let chip_path = system.parent().unwrap().join(&manifest.chip);
    (ChipDescriptor::from_file(chip_path).unwrap(), manifest)
}

#[test]
fn every_system_declaring_a_led_strip_builds() {
    let mut checked = Vec::new();
    for entry in std::fs::read_dir(root().join("configs/systems")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let Ok(manifest) = SystemManifest::from_file(&path) else {
            continue;
        };
        if !manifest
            .external_devices
            .iter()
            .any(|d| STRIP_TYPES.contains(&d.r#type.as_str()))
        {
            continue;
        }
        let (chip, manifest) = build(&path);
        SystemBus::from_config(&chip, &manifest)
            .unwrap_or_else(|e| panic!("{} does not build: {e:#}", path.display()));
        checked.push(path.file_name().unwrap().to_string_lossy().into_owned());
    }
    assert!(
        checked.iter().any(|n| n == "pybadge.yaml"),
        "the scan must reach the SAMD strip board; checked {checked:?}"
    );
}

#[test]
fn port_style_pads_resolve_on_samd_and_reject_undeclared_pins() {
    let (chip, manifest) = build(&root().join("configs/systems/pybadge.yaml"));
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let ext = manifest
        .external_devices
        .iter()
        .find(|d| d.r#type == "neopixel")
        .unwrap();
    let ctx = AttachCtx::new(&mut bus, ext);
    let pa15 = ctx.parse_gpio_pin("PA15").expect("PA15 is declared");
    let pb0 = ctx.parse_gpio_pin("PB0").expect("PB0 is declared");
    assert_eq!(pa15, 15);
    assert_eq!(pb0, 32, "next port block starts 32 pads up");
    assert_ne!(pa15, pb0);
    // Not declared by the chip: must not be misread as an ESP pad.
    assert_eq!(ctx.parse_gpio_pin("PA99"), None);
}
