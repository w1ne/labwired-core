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
const FDCAN_RX_FIFO0_ELEMENT_BYTES: u64 = 18 * 4;

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

fn interconnect(
    interconnect_type: &str,
    nodes: &[&str],
    config: HashMap<String, serde_yaml::Value>,
) -> InterconnectConfig {
    InterconnectConfig {
        r#type: interconnect_type.to_string(),
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
    quiet_can_environment_with_nodes(&["tester", "ecu"], interconnect)
}

fn quiet_can_environment_with_nodes(
    ids: &[&str],
    interconnect: InterconnectConfig,
) -> EnvironmentManifest {
    EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "quiet-h5-can-nodes".to_string(),
        nodes: ids.iter().map(|id| quiet_can_node(id)).collect(),
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
    // FDCAN1 is clock-gated on the H563. Firmware must enable its RCC gate
    // before taking FDCAN out of INIT.
    write_u32(machine, RCC_BASE + RCC_APB1HENR, 1 << 9);
    write_u32(machine, FDCAN_BASE + FDCAN_CCCR, 0);
}

fn queue_fdcan_tx(machine: &mut dyn MachineTrait, id: u32, payload: u8) {
    write_u32(machine, FDCAN_BASE + FDCAN_RAM + 0x278, id << 18);
    write_u32(machine, FDCAN_BASE + FDCAN_RAM + 0x27C, 1 << 16);
    write_u32(machine, FDCAN_BASE + FDCAN_RAM + 0x280, u32::from(payload));
    write_u32(machine, FDCAN_BASE + FDCAN_TXBAR, 1);
}

fn rx_fifo0_frame(machine: &dyn MachineTrait, slot: u64) -> (u32, u8) {
    let base = FDCAN_BASE + FDCAN_RAM + 0xB0 + slot * FDCAN_RX_FIFO0_ELEMENT_BYTES;
    (
        (read_u32(machine, base) >> 18) & 0x7FF,
        (read_u32(machine, base + 8) & 0xFF) as u8,
    )
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
    queue_fdcan_tx(
        world.machines.get_mut("tester").unwrap().as_mut(),
        0x321,
        0xA5,
    );

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
    assert_eq!(rx_fifo0_frame(receiver.as_ref(), 0), (0x321, 0xA5));
}

#[test]
fn manifest_can_bus_keeps_fdcan_reset_until_its_rcc_gate_is_enabled() {
    let mut world = World::from_manifest(
        quiet_can_environment(can_bus(&["tester", "ecu"], Some("fdcan1"))),
        &repo_root(),
    )
    .expect("the manifest-defined FDCAN topology should build");

    // The unclocked write must be dropped. Once the real RCC gate opens, the
    // controller still reports its reset INIT state rather than the requested
    // started state. This keeps firmware responsibility for RCC setup intact.
    let tester = world.machines.get_mut("tester").unwrap();
    write_u32(tester.as_mut(), FDCAN_BASE + FDCAN_CCCR, 0);
    write_u32(tester.as_mut(), RCC_BASE + RCC_APB1HENR, 1 << 9);
    assert_eq!(
        read_u32(tester.as_ref(), FDCAN_BASE + FDCAN_CCCR) & 1,
        1,
        "unclocked FDCAN writes must not take the controller out of INIT"
    );

    for id in ["tester", "ecu"] {
        start_fdcan(world.machines.get_mut(id).unwrap().as_mut());
    }
    queue_fdcan_tx(
        world.machines.get_mut("tester").unwrap().as_mut(),
        0x456,
        0x5A,
    );
    for _ in 0..2 {
        let results = world.step_all();
        assert!(
            results.values().all(Result::is_ok),
            "world round failed: {results:?}"
        );
    }

    let receiver = world.machines.get("ecu").unwrap();
    assert_eq!(
        read_u32(receiver.as_ref(), FDCAN_BASE + FDCAN_RXF0S) & 0x7F,
        1,
        "the real FDCAN wiring routes once firmware enables its RCC gate"
    );
    assert_eq!(rx_fifo0_frame(receiver.as_ref(), 0), (0x456, 0x5A));
}

#[test]
fn manifest_can_bus_delivery_order_is_lexical_not_yaml_order() {
    // The bus membership is deliberately reversed. The two transmitters queue
    // in the same World round and the observer must receive them in lexical
    // node-ID order, not the incidental `nodes:` order in this manifest.
    let mut world = World::from_manifest(
        quiet_can_environment_with_nodes(
            &["alpha", "observer", "zeta"],
            can_bus(&["zeta", "observer", "alpha"], Some("fdcan1")),
        ),
        &repo_root(),
    )
    .expect("the three-node manifest-defined FDCAN topology should build");

    for id in ["alpha", "observer", "zeta"] {
        start_fdcan(world.machines.get_mut(id).unwrap().as_mut());
    }

    queue_fdcan_tx(
        world.machines.get_mut("alpha").unwrap().as_mut(),
        0x111,
        0xA1,
    );
    queue_fdcan_tx(
        world.machines.get_mut("zeta").unwrap().as_mut(),
        0x222,
        0xB2,
    );

    for _ in 0..2 {
        let results = world.step_all();
        assert!(
            results.values().all(Result::is_ok),
            "world round failed: {results:?}"
        );
    }

    let observer = world.machines.get("observer").unwrap();
    assert_eq!(
        read_u32(observer.as_ref(), FDCAN_BASE + FDCAN_RXF0S) & 0x7F,
        2,
        "the passive observer receives both simultaneous sender frames"
    );
    assert_eq!(
        rx_fifo0_frame(observer.as_ref(), 0),
        (0x111, 0xA1),
        "the lexical-first sender is delivered first"
    );
    assert_eq!(
        rx_fifo0_frame(observer.as_ref(), 1),
        (0x222, 0xB2),
        "the lexical-last sender is delivered second"
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
        missing.contains("can_bus node 'ecu': no peripheral 'missing_fdcan'"),
        "{missing}"
    );

    let wrong_kind = world_error(environment(can_bus(&["tester", "ecu"], Some("rcc"))));
    assert!(
        wrong_kind.contains("can_bus node 'ecu': peripheral 'rcc' is not an FDCAN"),
        "{wrong_kind}"
    );
}

#[test]
fn uart_cross_link_requires_exactly_two_unique_known_nodes() {
    let third_member = world_error(quiet_can_environment_with_nodes(
        &["alpha", "beta", "gamma"],
        interconnect(
            "uart_cross_link",
            &["alpha", "beta", "gamma"],
            HashMap::new(),
        ),
    ));
    assert!(
        third_member.contains("uart_cross_link: requires exactly two unique nodes"),
        "{third_member}"
    );

    let duplicate = world_error(quiet_can_environment_with_nodes(
        &["alpha", "beta"],
        interconnect("uart_cross_link", &["alpha", "alpha"], HashMap::new()),
    ));
    assert!(
        duplicate.contains("uart_cross_link: requires exactly two unique nodes"),
        "{duplicate}"
    );

    let unknown = world_error(quiet_can_environment_with_nodes(
        &["alpha", "beta"],
        interconnect("uart_cross_link", &["alpha", "missing"], HashMap::new()),
    ));
    assert!(
        unknown.contains("uart_cross_link: unknown node 'missing'"),
        "{unknown}"
    );
}

#[test]
fn egress_requires_exactly_one_known_node() {
    let mut config = HashMap::new();
    config.insert(
        "url".to_string(),
        serde_yaml::Value::String("127.0.0.1:9".to_string()),
    );
    config.insert(
        "uart".to_string(),
        serde_yaml::Value::String("uart2".to_string()),
    );
    let extra_member = world_error(environment(interconnect(
        "egress",
        &["tester", "ecu"],
        config,
    )));
    assert!(
        extra_member.contains("egress: requires exactly one node"),
        "{extra_member}"
    );
}
