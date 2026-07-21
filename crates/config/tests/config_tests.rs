// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_config::{ChipDescriptor, CosimAdapter, MemoryValueDetails, SystemManifest};

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
fn system_manifest_parses_cosim_models() {
    let yaml = r#"
name: "plant-demo"
chip: "chips/stm32f103.yaml"
cosim_models:
  - id: "plant_model"
    adapter: "external_process"
    model: "./models/plant.jsonl"
    step_ns: 10000
    inputs:
      rem0_enable: "gpio.rem0"
      rem1_enable: "gpio.rem1"
    outputs:
      v_out: "scope.channel_a"
      i_out: "meter.output_current"
    config:
      protocol: "jsonl"
external_devices: []
"#;

    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();

    assert_eq!(manifest.cosim_models.len(), 1);
    let model = &manifest.cosim_models[0];
    assert_eq!(model.id, "plant_model");
    assert_eq!(model.adapter, CosimAdapter::ExternalProcess);
    assert_eq!(model.model.as_deref(), Some("./models/plant.jsonl"));
    assert_eq!(model.step_ns, 10_000);
    assert_eq!(model.inputs["rem0_enable"], "gpio.rem0");
    assert_eq!(model.outputs["v_out"], "scope.channel_a");
    assert_eq!(
        model.config["protocol"],
        serde_yaml::Value::String("jsonl".to_string())
    );
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
fn system_manifest_rejects_incomplete_cosim_model() {
    let yaml = r#"
name: "bad-cosim"
chip: "chips/stm32f103.yaml"
cosim_models:
  - id: ""
    adapter: "fmi"
    step_ns: 0
external_devices: []
"#;

    let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
    let issues = manifest.validate_cosim_models();

    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("cosim_models[0].id")),
        "expected missing id validation issue, got {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("cosim_models[0].model")),
        "expected missing model validation issue, got {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("cosim_models[0].step_ns")),
        "expected invalid step validation issue, got {issues:?}"
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
