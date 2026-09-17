use super::*;

#[test]
fn chip_descriptor_parses_pins_and_ignores_extra_fields() {
    let yaml = r#"
name: "test-chip"
arch: "arm"
flash: { base: 0, size: "64KB" }
ram: { base: 0x20000000, size: "16KB" }
peripherals: []
pins:
  PC0: { gpio: gpioc, bit: 0, functions: [{ type: gpio, peripheral: gpioc }] }
  PB6: { gpio: gpioc, bit: 2 }
"#;
    let chip: ChipDescriptor = serde_yaml::from_str(yaml).expect("parse chip with pins");
    assert_eq!(chip.pins.len(), 2);
    assert_eq!(chip.pins["PC0"].gpio, "gpioc");
    assert_eq!(chip.pins["PC0"].bit, 0);
    // `functions:` in the YAML is ignored by PinLoc (serde ignores unknown fields).
    assert_eq!(chip.pins["PB6"].gpio, "gpioc");
    assert_eq!(chip.pins["PB6"].bit, 2);
}

#[test]
fn chip_without_pins_defaults_to_empty() {
    let yaml = r#"
name: "no-pins"
arch: "arm"
flash: { base: 0, size: "64KB" }
ram: { base: 0x20000000, size: "16KB" }
peripherals: []
"#;
    let chip: ChipDescriptor = serde_yaml::from_str(yaml).expect("parse");
    assert!(chip.pins.is_empty());
}
