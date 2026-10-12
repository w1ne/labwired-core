// SPDX-License-Identifier: MIT
// Baseline value contracts; real Rust execution is qualified by CoreCI.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn inputs() -> (ChipDescriptor, SystemManifest) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32l476.yaml");
    let chip = ChipDescriptor::from_file(&path).unwrap();
    let manifest = serde_yaml::from_str(&format!(
        "name: ordered-metadata\nchip: '{}'\n",
        path.display()
    ))
    .unwrap();
    (chip, manifest)
}

#[test]
fn all_58_descriptors_and_949_registers_match_frozen_source_golden() {
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ordered-l476-debug-schemas.json")).unwrap();
    assert_eq!(expected["descriptor_count"], 58);
    assert_eq!(expected["register_count"], 949);
    let (mut chip, manifest) = inputs();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let model_order = bus
        .peripherals
        .iter()
        .map(|p| p.name.clone())
        .collect::<Vec<_>>();
    for reverse in [false, true] {
        if reverse {
            chip.peripherals.reverse();
        }
        bus.attach_debug_schemas(&chip, &manifest);
        assert_eq!(
            bus.peripherals
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>(),
            model_order
        );
        assert_eq!(bus.debug_schemas.len(), 58);
        assert_eq!(bus.debug_schemas.values().map(Vec::len).sum::<usize>(), 949);
        assert_eq!(
            serde_json::to_value(&bus.debug_schemas).unwrap(),
            expected["schemas"]
        );
    }
}

// Exclusive, test-owned filesystem fixture; never reuse or clean another path.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "lw-ordered-debug-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("bad.yaml"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[test]
fn loader_keeps_last_valid_duplicate_skips_bad_and_replaces_on_attach() {
    let (mut chip, manifest) = inputs();
    let original = chip.peripherals.clone();
    let mut first = original.iter().find(|p| p.id == "rcc").unwrap().clone();
    let mut last = original.iter().find(|p| p.id == "gpioa").unwrap().clone();
    first.id = "same".into();
    last.id = "same".into();
    let scratch = Scratch::new();
    let bad_path = scratch.0.join("bad.yaml");
    std::fs::write(
        &bad_path,
        "peripheral: BAD\nversion: v1\nregisters: []\ninterrupts: {BAD: -1}\n",
    )
    .unwrap();
    let mut bad = last.clone();
    bad.config.insert(
        "debug_schema".into(),
        serde_yaml::Value::String(bad_path.to_str().unwrap().into()),
    );
    let mut missing = last.clone();
    missing.id = "missing".into();
    missing.config.insert(
        "debug_schema".into(),
        serde_yaml::Value::String(scratch.0.join("absent.yaml").to_str().unwrap().into()),
    );
    chip.peripherals = vec![first, last, bad, missing];
    let mut bus = SystemBus::new();
    bus.attach_debug_schemas(&chip, &manifest);
    assert_eq!(bus.debug_schemas.len(), 1);
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ordered-l476-debug-schemas.json")).unwrap();
    assert_eq!(
        serde_json::to_value(&bus.debug_schemas["same"]).unwrap(),
        expected["schemas"]["gpioa"]
    );
    assert!(!bus.debug_schemas.contains_key("missing"));
    chip.peripherals.clear();
    bus.attach_debug_schemas(&chip, &manifest);
    assert!(bus.debug_schemas.is_empty());
}
