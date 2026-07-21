use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;

fn bus_for(chip_file: &str) -> SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips")
        .join(chip_file);
    let chip = ChipDescriptor::from_file(&path).expect("load chip");
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "pinmap".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("assemble bus")
}

#[test]
fn mkw41z4_pc0_resolves_via_pin_map() {
    let bus = bus_for("mkw41z4.yaml");
    // PC0 → gpioc (base 0x400FF080) + Kinetis PDOR offset 0x00, bit 0.
    let (addr, bit) = SystemBus::resolve_pin_odr_pub(&bus, "PC0").expect("PC0 resolves");
    assert_eq!(addr, 0x400F_F080);
    assert_eq!(bit, 0);
}

#[test]
fn mkw41z4_pb6_maps_to_gpioc_not_gpiob() {
    // The disagreement this whole change fixes: label letter 'B' would parse to
    // gpiob, but the chip map says PB6 is gpioc bit 2.
    let bus = bus_for("mkw41z4.yaml");
    let (addr, bit) = SystemBus::resolve_pin_odr_pub(&bus, "PB6").expect("PB6 resolves");
    assert_eq!(addr, 0x400F_F080); // gpioc, NOT gpiob (0x400FF040)
    assert_eq!(bit, 2);
}

#[test]
fn mkw41z4_undeclared_pin_fails_loudly_no_letter_parse() {
    // PB0 is not in mkw41z4 `pins:`. With a declared map, resolution must NOT
    // fall back to parsing 'B' → gpiob. It returns None (caller errors).
    let bus = bus_for("mkw41z4.yaml");
    assert!(SystemBus::resolve_pin_odr_pub(&bus, "PB0").is_none());
}

#[test]
fn chip_without_pin_map_still_letter_parses() {
    // Regression: stm32f103 declares no `pins:`, so the standard-layout parse
    // still resolves PC13 → gpioc.
    let bus = bus_for("stm32f103.yaml");
    let resolved = SystemBus::resolve_pin_odr_pub(&bus, "PC13");
    assert!(
        resolved.is_some(),
        "PC13 must resolve on stm32f103 via label parse"
    );
}
