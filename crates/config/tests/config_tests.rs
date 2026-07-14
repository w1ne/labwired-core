// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_config::{ChipDescriptor, MemoryValueDetails};

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

#[test]
fn memory_value_details_constructor_is_externally_constructible_and_sparse() {
    let details = MemoryValueDetails::new(0x2001_0000, 1);
    assert_eq!(details.mask, None);
    assert_eq!(details.size, None);
    assert_eq!(details.node, None);

    let serialized = serde_yaml::to_string(&details).unwrap();
    assert!(
        !serialized.contains("node:"),
        "ordinary node-less details should stay sparse: {serialized}"
    );
}

#[test]
fn memory_value_details_public_fields_remain_struct_literal_constructible() {
    // This is compiled as a downstream crate. Keep the public struct shape
    // usable by callers that construct a memory assertion directly.
    let details = MemoryValueDetails {
        address: 0x2001_0000,
        expected_value: 1,
        mask: None,
        size: None,
        node: None,
    };

    let serialized = serde_yaml::to_string(&details).unwrap();
    assert!(
        !serialized.contains("node:"),
        "ordinary node-less details should stay sparse: {serialized}"
    );
}
