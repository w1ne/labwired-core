use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
use labwired_core::world::World;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node(id: &str) -> NodeConfig {
    NodeConfig {
        id: id.to_string(),
        system: "examples/h563-uds-ecu/system.yaml".to_string(),
        firmware: "examples/h563-uds-ecu/firmware/h563_uds_ecu.elf".to_string(),
        config_overrides: HashMap::new(),
    }
}

fn can_bus(nodes: &[&str], peripheral: Option<&str>) -> InterconnectConfig {
    let mut config = HashMap::new();
    if let Some(peripheral) = peripheral {
        config.insert(
            "peripheral".to_string(),
            serde_yaml::Value::String(peripheral.to_string()),
        );
    }
    InterconnectConfig {
        r#type: "can_bus".to_string(),
        nodes: nodes.iter().map(|node| (*node).to_string()).collect(),
        config,
    }
}

fn environment(interconnect: InterconnectConfig) -> EnvironmentManifest {
    EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "two-h5-nodes".to_string(),
        nodes: vec![node("tester"), node("ecu")],
        interconnects: vec![interconnect],
    }
}

fn world_error(environment: EnvironmentManifest) -> String {
    match World::from_manifest(environment, &repo_root()) {
        Ok(_) => panic!("manifest should be rejected"),
        Err(error) => format!("{error:#}"),
    }
}

#[test]
fn from_manifest_wires_can_bus_to_each_named_fdcan_and_configures_cortex_m() {
    let world = World::from_manifest(
        environment(can_bus(&["tester", "ecu"], Some("fdcan1"))),
        &repo_root(),
    )
    .expect("a two-node FDCAN bus should build");

    assert_eq!(world.interconnects.len(), 1);
    for id in ["tester", "ecu"] {
        // SCB.CPUID low byte proves from_manifest used configure_cortex_m()
        // rather than a bare CortexM::new() with an unmapped system-control
        // block.
        assert_eq!(
            world.machines[id].read_u8(0xE000_ED00).unwrap(),
            0x41,
            "{id} gets the normal Cortex-M system wiring"
        );
    }
}

#[test]
fn can_bus_requires_a_nonblank_named_peripheral_and_two_unique_nodes() {
    let missing = world_error(environment(can_bus(&["tester", "ecu"], None)));
    assert!(
        missing.contains("can_bus: missing nonblank config.peripheral"),
        "{missing}"
    );

    let blank = world_error(environment(can_bus(&["tester", "ecu"], Some("  "))));
    assert!(
        blank.contains("can_bus: missing nonblank config.peripheral"),
        "{blank}"
    );

    let one_node = world_error(environment(can_bus(&["tester"], Some("fdcan1"))));
    assert!(
        one_node.contains("can_bus: requires at least two unique nodes"),
        "{one_node}"
    );

    let duplicate = world_error(environment(can_bus(&["tester", "tester"], Some("fdcan1"))));
    assert!(
        duplicate.contains("can_bus: requires at least two unique nodes"),
        "{duplicate}"
    );
}

#[test]
fn can_bus_reports_unknown_node_and_missing_or_non_fdcan_peripheral() {
    let unknown = world_error(environment(can_bus(
        &["tester", "not-a-node"],
        Some("fdcan1"),
    )));
    assert!(
        unknown.contains("can_bus: unknown node 'not-a-node'"),
        "{unknown}"
    );

    let missing = world_error(environment(can_bus(
        &["tester", "ecu"],
        Some("missing_fdcan"),
    )));
    assert!(
        missing.contains("can_bus node 'tester': no peripheral 'missing_fdcan'"),
        "{missing}"
    );

    let wrong_kind = world_error(environment(can_bus(&["tester", "ecu"], Some("rcc"))));
    assert!(
        wrong_kind.contains("can_bus node 'tester': peripheral 'rcc' is not an FDCAN"),
        "{wrong_kind}"
    );
}
