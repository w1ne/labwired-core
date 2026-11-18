use labwired_config::SystemManifest;
use labwired_core::inspect::InspectOpts;
use labwired_core::peripherals::components::{
    i2c_factory::build_i2c_tree, max30102::Max30102, tca9548a::Tca9548a,
};
fn manifest(powered: bool) -> SystemManifest {
    serde_yaml::from_str(&format!(r#"
name: factory-identity-contract
chip: inline
external_devices:
  - {{id: mux, type: tca9548a, connection: i2c1, config: {{i2c_address: 0x71}}}}
  - {{id: pca, type: pca9685, connection: mux, channel: 0, config: {{i2c_address: 0x40, powered: {powered}}}}}
  - {{id: sensor, type: max30102, connection: i2c1, config: {{i2c_address: 0x57, powered: {powered}}}}}
"#)).unwrap()
}
#[test]
fn factory_wrappers_preserve_identity_power_sim_input_and_mux_inspection() {
    for powered in [false, true] {
        let manifest = manifest(powered);
        let mut sensor = build_i2c_tree(&manifest, &manifest.external_devices[2])
            .unwrap()
            .unwrap();
        assert_eq!(sensor.i2c_component_id(), Some("sensor"));
        assert_eq!(sensor.claims_address(0x57), powered);
        assert!(sensor.as_any().unwrap().is::<Max30102>());
        assert_eq!(
            sensor.as_sim_input_mut().unwrap().component_id(),
            Some("sensor")
        );
        let switch = build_i2c_tree(&manifest, &manifest.external_devices[0])
            .unwrap()
            .unwrap();
        assert_eq!(switch.i2c_component_id(), Some("mux"));
        assert!(switch.is_i2c_mux());
        let mux = switch
            .as_any()
            .unwrap()
            .downcast_ref::<Tca9548a>()
            .expect("mux downcast through wrapper");
        let child = &mux.channel_devices(0)[0];
        assert_eq!(child.i2c_component_id(), Some("pca"));
        assert_eq!(child.claims_address(0x40), powered);
        let artifacts = child.artifacts("pca", &InspectOpts::default());
        assert!(!artifacts.is_empty(), "PCA file is inspectable");
        assert_eq!(
            artifacts[0].meta["powered"].as_bool(),
            Some(powered),
            "nested inspection must reflect actual power"
        );
    }
}
