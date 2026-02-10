// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_config::ChipDescriptor;

#[test]
fn test_old_yaml_still_parses() {
    let yaml = r#"
name: "test-chip"
arch: "cortex-m3"
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals:
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
"#;
    let desc: ChipDescriptor = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(desc.peripherals.len(), 1);
    assert_eq!(desc.peripherals[0].id, "uart1");
    assert_eq!(desc.peripherals[0].size, None);
    assert_eq!(desc.peripherals[0].irq, None);
}

#[test]
fn test_new_fields_parse() {
    let yaml = r#"
name: "test-chip"
arch: "cortex-m3"
flash:
  base: 0x0
  size: "1MB"
ram:
  base: 0x20000000
  size: "128KB"
peripherals:
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    irq: 37
"#;
    let desc: ChipDescriptor = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(desc.peripherals.len(), 1);
    assert_eq!(desc.peripherals[0].id, "uart1");
    assert_eq!(desc.peripherals[0].size, Some("1KB".to_string()));
    assert_eq!(desc.peripherals[0].irq, Some(37));
}
