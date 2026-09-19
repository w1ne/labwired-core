//! Descriptor resolution must not depend on whether a sensor sits behind a mux.

use labwired_config::{DeviceDescriptor, SystemManifest};
use labwired_core::peripherals::components::{build_i2c_tree, validate_i2c_mux_topology};
use labwired_core::peripherals::i2c::I2cDevice;

fn manifest(device_type: &str) -> SystemManifest {
    SystemManifest::from_yaml(&format!(
        r#"
name: yaml-mux-resolution
chip: stm32f103
external_devices:
  - {{ id: outer, type: tca9548a, connection: i2c1 }}
  - id: inner
    type: tca9548a
    connection: outer
    channel: 2
    config: {{ i2c_address: 0x71 }}
  - id: sensor
    type: "{device_type}"
    connection: inner
    channel: 3
    config: {{ i2c_address: 0x4a, temperature: 42 }}
"#
    ))
    .unwrap()
}

fn write(device: &mut dyn I2cDevice, address: u8, bytes: &[u8]) {
    assert!(device.claims_address(address), "no ACK at {address:#x}");
    device.select_address(address);
    device.start();
    for byte in bytes {
        device.write(*byte);
    }
    device.stop();
}

fn read(device: &mut dyn I2cDevice, address: u8, register: u8, count: usize) -> Vec<u8> {
    write(device, address, &[register]);
    device.select_address(address);
    device.start();
    let bytes = (0..count).map(|_| device.read()).collect();
    device.stop();
    bytes
}

fn connected_tree(manifest: &SystemManifest) -> Box<dyn I2cDevice> {
    assert_eq!(
        validate_i2c_mux_topology(manifest).unwrap(),
        ["inner", "sensor"]
    );
    let mut tree = build_i2c_tree(manifest, &manifest.external_devices[0])
        .unwrap()
        .unwrap();
    assert!(!tree.claims_address(0x4a));
    write(tree.as_mut(), 0x70, &[1 << 2]);
    write(tree.as_mut(), 0x71, &[1 << 3]);
    assert!(tree.claims_address(0x4a));
    tree
}

#[test]
fn embedded_yaml_sensor_works_behind_nested_muxes() {
    let manifest = manifest("mcp9808");
    let mut tree = connected_tree(&manifest);
    assert_eq!(read(tree.as_mut(), 0x4a, 6, 2), [0, 0x54]);
    assert_eq!(read(tree.as_mut(), 0x4a, 7, 2), [4, 0]);
}

#[test]
fn every_embedded_i2c_descriptor_is_constructible_without_a_type_allowlist() {
    for yaml in labwired_config::embedded_device_yamls() {
        let descriptor = DeviceDescriptor::from_yaml(yaml).unwrap();
        if descriptor.behavior.primitive != "i2c_device" {
            continue;
        }
        let manifest = manifest(&descriptor.r#type);
        let result = validate_i2c_mux_topology(&manifest);
        assert!(result.is_ok(), "{}: {result:?}", descriptor.r#type);
        let mut tree = connected_tree(&manifest);
        let mut ids = Vec::new();
        tree.for_each_sim_input(&mut |input| {
            ids.push(input.component_id().map(str::to_owned));
            false
        });
        assert!(
            ids.contains(&Some("sensor".into())),
            "{}: {ids:?}",
            descriptor.r#type
        );
    }
}

fn private_pack(device_type: &str, overrides: bool) -> String {
    format!(
        r#"
schema: labwired.part/v1
type: "{device_type}"
{}
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x48
    registers:
      - {{ name: TEMP, addr: 0, width: 2, endian: be, access: r, source: temperature }}
metadata:
  inputs:
    - {{ key: temperature, label: Temperature, unit: C, min: 0, max: 100, default: 7 }}
"#,
        if overrides {
            format!("overrides: {device_type}")
        } else {
            String::new()
        }
    )
}

fn with_pack(mut manifest: SystemManifest, yaml: &str) -> SystemManifest {
    manifest.parts.push(serde_yaml::from_str(yaml).unwrap());
    manifest
}

#[test]
fn private_yaml_sensor_preserves_config_and_identity_behind_muxes() {
    let manifest = with_pack(
        manifest("acme:thermometer"),
        &private_pack("acme:thermometer", false),
    );
    let mut tree = connected_tree(&manifest);
    assert_eq!(read(tree.as_mut(), 0x4a, 0, 2), [0, 42]);
    tree.for_each_sim_input(&mut |input| {
        assert_eq!(input.component_id(), Some("sensor"));
        input.set_input("temperature", 57.0).unwrap();
        false
    });
    assert_eq!(read(tree.as_mut(), 0x4a, 0, 2), [0, 57]);
}

#[test]
fn private_yaml_sensor_honors_noise_configuration_behind_muxes() {
    let yaml = private_pack("acme:thermometer", false).replace(
        "default: 7",
        "default: 7, noise_sigma: 20, noise_sigma_key: sensor_noise",
    );
    let mut manifest = with_pack(manifest("acme:thermometer"), &yaml);
    manifest.external_devices[2]
        .config
        .insert("sensor_noise".into(), 0.into());
    let mut tree = connected_tree(&manifest);
    for _ in 0..16 {
        assert_eq!(read(tree.as_mut(), 0x4a, 0, 2), [0, 42]);
    }
}

#[test]
fn explicit_override_has_the_same_precedence_behind_muxes() {
    let manifest = with_pack(manifest("mcp9808"), &private_pack("mcp9808", true));
    let mut tree = connected_tree(&manifest);
    assert_eq!(read(tree.as_mut(), 0x4a, 0, 2), [0, 42]);
}

#[test]
fn implicit_override_is_rejected_before_tree_construction() {
    let manifest = with_pack(manifest("tmp102"), &private_pack("tmp102", false));
    let error = validate_i2c_mux_topology(&manifest).unwrap_err();
    assert!(
        format!("{error:#}").contains("shadows a built-in part"),
        "{error:#}"
    );
}

#[test]
fn wrong_transport_override_does_not_fall_back_to_builtin_sensor() {
    let manifest = with_pack(
        manifest("tmp102"),
        r#"
schema: labwired.part/v1
type: tmp102
overrides: tmp102
behavior:
  primitive: analog_source
  analog:
    curve: [[0, 0], [100, 3300]]
metadata:
  inputs:
    - { key: temperature, label: Temperature, unit: C, min: 0, max: 100 }
"#,
    );
    assert!(validate_i2c_mux_topology(&manifest).is_err());
    assert!(build_i2c_tree(&manifest, &manifest.external_devices[0]).is_err());
}

#[test]
fn unpowered_sensor_behind_mux_does_not_acknowledge() {
    let mut manifest = manifest("tmp102");
    manifest.external_devices[2]
        .config
        .insert("powered".into(), false.into());
    validate_i2c_mux_topology(&manifest).unwrap();
    let mut tree = build_i2c_tree(&manifest, &manifest.external_devices[0])
        .unwrap()
        .unwrap();
    write(tree.as_mut(), 0x70, &[1 << 2]);
    write(tree.as_mut(), 0x71, &[1 << 3]);
    assert!(!tree.claims_address(0x4a), "an unpowered sensor must NACK");
}

#[test]
fn invalid_address_is_rejected_instead_of_truncated() {
    let mut manifest = manifest("tmp102");
    manifest.external_devices[2]
        .config
        .insert("i2c_address".into(), 0x14a.into());
    let error = validate_i2c_mux_topology(&manifest).unwrap_err();
    assert!(format!("{error:#}").contains("i2c_address"), "{error:#}");
}

#[test]
fn invalid_descriptor_default_address_is_rejected() {
    let mut manifest = with_pack(
        manifest("acme:thermometer"),
        &private_pack("acme:thermometer", false)
            .replace("default_address: 0x48", "default_address: 0x80"),
    );
    manifest.external_devices[2].config.remove("i2c_address");
    let error = validate_i2c_mux_topology(&manifest).unwrap_err();
    assert!(format!("{error:#}").contains("address"), "{error:#}");
}

#[test]
fn replacing_a_mux_with_a_sensor_cannot_silently_drop_its_children() {
    let manifest = with_pack(manifest("tmp102"), &private_pack("tca9548a", true));
    assert!(validate_i2c_mux_topology(&manifest).is_err());
    assert!(build_i2c_tree(&manifest, &manifest.external_devices[0]).is_err());
}

#[test]
fn wrong_transport_mux_override_is_rejected_by_direct_tree_construction() {
    let manifest = with_pack(
        manifest("tmp102"),
        r#"
schema: labwired.part/v1
type: tca9548a
overrides: tca9548a
behavior:
  primitive: analog_source
  analog:
    curve: [[0, 0], [100, 3300]]
metadata:
  inputs:
    - { key: temperature, label: Temperature, unit: C, min: 0, max: 100 }
"#,
    );
    assert!(validate_i2c_mux_topology(&manifest).is_err());
    assert!(build_i2c_tree(&manifest, &manifest.external_devices[0]).is_err());
}

#[test]
fn an_unpowered_switch_hides_all_of_its_downstream_devices() {
    let mut manifest = manifest("tmp102");
    manifest.external_devices[1]
        .config
        .insert("powered".into(), false.into());
    validate_i2c_mux_topology(&manifest).unwrap();
    let mut tree = build_i2c_tree(&manifest, &manifest.external_devices[0])
        .unwrap()
        .unwrap();
    write(tree.as_mut(), 0x70, &[1 << 2]);
    assert!(!tree.claims_address(0x71));
    assert!(!tree.claims_address(0x4a));
}
