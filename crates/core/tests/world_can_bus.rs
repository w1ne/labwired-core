use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
use labwired_core::world::{MachineTrait, World};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const RCC_BASE: u64 = 0x4402_0C00;
const RCC_APB1HENR: u64 = 0x0A0;
const FDCAN_BASE: u64 = 0x4000_A400;
const FDCAN_RAM: u64 = 0x800;
const FDCAN_CCCR: u64 = 0x018;
const FDCAN_RXF0S: u64 = 0x090;
const FDCAN_TXBAR: u64 = 0x0CC;

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

fn quiet_can_node(id: &str) -> NodeConfig {
    NodeConfig {
        id: id.to_string(),
        // This deliberately contains no synthetic in-process UDS tester: the
        // only CAN traffic in the end-to-end test below must traverse the
        // manifest-defined interconnect between the two machines.
        system: "crates/core/tests/fixtures/h563-can-world-system.yaml".to_string(),
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

fn quiet_can_environment(interconnect: InterconnectConfig) -> EnvironmentManifest {
    EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "two-quiet-h5-nodes".to_string(),
        nodes: vec![quiet_can_node("tester"), quiet_can_node("ecu")],
        interconnects: vec![interconnect],
    }
}

fn world_error(environment: EnvironmentManifest) -> String {
    match World::from_manifest(environment, &repo_root()) {
        Ok(_) => panic!("manifest should be rejected"),
        Err(error) => format!("{error:#}"),
    }
}

fn write_u32(machine: &mut dyn MachineTrait, addr: u64, value: u32) {
    for (byte, value) in value.to_le_bytes().into_iter().enumerate() {
        machine.write_u8(addr + byte as u64, value).unwrap();
    }
}

fn read_u32(machine: &dyn MachineTrait, addr: u64) -> u32 {
    let bytes = std::array::from_fn(|byte| machine.read_u8(addr + byte as u64).unwrap());
    u32::from_le_bytes(bytes)
}

fn start_fdcan(machine: &mut dyn MachineTrait) {
    // FDCAN1 is clock-gated on the H563. Mirror the real firmware's RCC
    // enable before taking FDCAN out of INIT.
    write_u32(machine, RCC_BASE + RCC_APB1HENR, 1 << 9);
    write_u32(machine, FDCAN_BASE + FDCAN_CCCR, 0);
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
fn manifest_can_bus_routes_a_known_fdcan_frame_without_sender_echo() {
    let mut world = World::from_manifest(
        quiet_can_environment(can_bus(&["tester", "ecu"], Some("fdcan1"))),
        &repo_root(),
    )
    .expect("the manifest-defined FDCAN topology should build");

    for id in ["tester", "ecu"] {
        start_fdcan(world.machines.get_mut(id).unwrap().as_mut());
    }

    // Program TX buffer 0 on the tester only. All register accesses go through
    // World -> MachineTrait -> SystemBus; no isolated FDCAN or CanBus endpoint
    // is constructed in this test.
    let sender = world.machines.get_mut("tester").unwrap();
    write_u32(sender.as_mut(), FDCAN_BASE + FDCAN_RAM + 0x278, 0x321 << 18);
    write_u32(sender.as_mut(), FDCAN_BASE + FDCAN_RAM + 0x27C, 1 << 16);
    write_u32(sender.as_mut(), FDCAN_BASE + FDCAN_RAM + 0x280, 0x0000_00A5);
    write_u32(sender.as_mut(), FDCAN_BASE + FDCAN_TXBAR, 1);

    // Round 1 drains the sender into the manifest-wired CanBus. In round 2 the
    // lexical-first ECU drains the endpoint before the tester takes its turn.
    for _ in 0..2 {
        let results = world.step_all();
        assert!(
            results.values().all(Result::is_ok),
            "world round failed: {results:?}"
        );
    }

    let sender = world.machines.get("tester").unwrap();
    assert_eq!(
        read_u32(sender.as_ref(), FDCAN_BASE + FDCAN_RXF0S) & 0x7F,
        0,
        "the transmitting node must not receive its own frame"
    );

    let receiver = world.machines.get("ecu").unwrap();
    assert_eq!(
        read_u32(receiver.as_ref(), FDCAN_BASE + FDCAN_RXF0S) & 0x7F,
        1,
        "the distinct manifest-connected node receives exactly one frame"
    );
    assert_eq!(
        (read_u32(receiver.as_ref(), FDCAN_BASE + FDCAN_RAM + 0xB0) >> 18) & 0x7FF,
        0x321
    );
    assert_eq!(
        (read_u32(receiver.as_ref(), FDCAN_BASE + FDCAN_RAM + 0xB4) >> 16) & 0xF,
        1
    );
    assert_eq!(
        read_u32(receiver.as_ref(), FDCAN_BASE + FDCAN_RAM + 0xB8) & 0xFF,
        0xA5
    );
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
