use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
use labwired_core::world::{MachineTrait, World};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const RCC_BASE: u64 = 0x4402_0C00;
const RCC_APB1HENR: u64 = 0x0A0;
const FDCAN_BASE: u64 = 0x4000_A400;
const FDCAN_RAM: u64 = 0x800;
const FDCAN_CCCR: u64 = 0x018;
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
        profile: None,
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
        profile: None,
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

fn can_bus_with_endpoints(nodes: &[&str], endpoints: &[(&str, &str)]) -> InterconnectConfig {
    let endpoints = endpoints
        .iter()
        .map(|(node, endpoint)| {
            (
                serde_yaml::Value::String((*node).to_string()),
                serde_yaml::Value::String((*endpoint).to_string()),
            )
        })
        .collect();
    InterconnectConfig {
        r#type: "can_bus".to_string(),
        nodes: nodes.iter().map(|node| (*node).to_string()).collect(),
        config: HashMap::from([(
            "endpoints".to_string(),
            serde_yaml::Value::Mapping(endpoints),
        )]),
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
        rf: None,
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
        rf: None,
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

/// Run a timed-CAN world for 1 ms of world time.
fn run_world(world: &mut World) {
    let results = world
        .run_until_ps(1_000_000_000)
        .expect("a world with a timed can_bus has a round clock");
    assert!(
        results.values().all(Result::is_ok),
        "world round failed: {results:?}"
    );
}

/// CAN frames a node's controller took into its receive FIFO, from the
/// node's bus trace, as `(id, first payload byte)`. The nodes' UDS firmware
/// polls and releases RX FIFO0 itself, so the FIFO registers cannot show
/// what arrived once the world has run for a while.
fn received(world: &World, id: &str) -> Vec<(u32, u8)> {
    use labwired_core::bus::bus_trace::{BusDir, BusPayload};
    world.machines[id]
        .bus_trace_snapshot()
        .into_iter()
        .filter_map(|event| match event.payload {
            BusPayload::Can {
                direction: BusDir::Rx,
                id,
                data,
                ..
            } => Some((id, data.first().copied().unwrap_or(0))),
            _ => None,
        })
        .collect()
}

#[test]
fn from_manifest_wires_can_bus_to_each_named_fdcan_and_configures_cortex_m() {
    let world = World::from_manifest(
        environment(can_bus(&["tester", "ecu"], Some("fdcan1"))),
        &repo_root(),
    )
    .expect("a two-node FDCAN bus should build");

    // Two FDCANs: the bus is timed, not an untimed fan-out interconnect.
    assert!(world.interconnects.is_empty());
    let buses = world.can_bus_reports();
    assert_eq!(buses.len(), 1);
    assert!(buses[0].timed);
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
fn can_bus_accepts_mcu_and_nested_spi_endpoints() {
    let fixture_dir = std::env::temp_dir().join(format!(
        "labwired-mixed-can-endpoints-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let scanner_system = fixture_dir.join("scanner-system.yaml");
    let chip = repo_root().join("configs/chips/stm32h563.yaml");
    std::fs::write(
        &scanner_system,
        format!(
            r#"name: mixed-can-scanner
chip: "{}"
external_devices:
  - id: mcp2515
    type: mcp2515
    connection: spi1
    config:
      cs_pin: P0.04
"#,
            chip.display()
        ),
    )
    .unwrap();

    let firmware = repo_root().join("examples/h563-uds-ecu/firmware/h563_uds_ecu.elf");
    let manifest = EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "mixed-mcu-and-spi-can".to_string(),
        nodes: vec![
            NodeConfig {
                id: "scanner".to_string(),
                system: scanner_system.to_string_lossy().into_owned(),
                firmware: firmware.to_string_lossy().into_owned(),
                config_overrides: HashMap::new(),
                profile: None,
            },
            NodeConfig {
                id: "ecu".to_string(),
                system: "examples/f103-j1939-monitor/system.yaml".to_string(),
                firmware: "examples/f103-fidelity-bench/firmware/build/gpiobug.elf".to_string(),
                config_overrides: HashMap::new(),
                profile: None,
            },
        ],
        interconnects: vec![can_bus_with_endpoints(
            &["scanner", "ecu"],
            &[("scanner", "mcp2515"), ("ecu", "bxcan1")],
        )],
        rf: None,
    };

    let serialized = serde_yaml::to_string(&manifest).unwrap();
    let parsed: EnvironmentManifest = serde_yaml::from_str(&serialized).unwrap();
    let world = World::from_manifest(parsed, &repo_root())
        .expect("mixed MCU and nested SPI CAN endpoints should build");
    assert_eq!(world.interconnects.len(), 1);
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

    // The bus is timed: the frame takes its wire time (about 100 µs at the
    // 500 kbit/s the reset NBTP gives from the 8 MHz kernel clock).
    run_world(&mut world);

    assert!(
        received(&world, "tester").is_empty(),
        "the transmitting node must not receive its own frame"
    );
    assert_eq!(
        received(&world, "ecu"),
        vec![(0x321, 0xA5)],
        "the distinct manifest-connected node receives exactly one frame"
    );
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
    run_world(&mut world);

    assert_eq!(
        received(&world, "ecu"),
        vec![(0x456, 0x5A)],
        "the real FDCAN wiring routes once firmware enables its RCC gate"
    );
}

#[test]
fn manifest_can_bus_delivery_order_is_arbitration_order_not_yaml_order() {
    // The bus membership is deliberately reversed. The two transmitters queue
    // at the same instant, so the frames contend for the bus and the lower
    // identifier wins arbitration, whatever the `nodes:` order says.
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

    run_world(&mut world);

    assert_eq!(
        received(&world, "observer"),
        vec![(0x111, 0xA1), (0x222, 0xB2)],
        "the lower identifier wins arbitration; the loser retransmits after it"
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
        missing.contains("can_bus node 'ecu': no CAN endpoint 'missing_fdcan'"),
        "{missing}"
    );

    let wrong_kind = world_error(environment(can_bus(&["tester", "ecu"], Some("rcc"))));
    assert!(
        wrong_kind.contains("can_bus node 'ecu': peripheral 'rcc' is not a CAN controller"),
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
fn world_rejects_unknown_interconnect_config_before_topology_build() {
    let mut config = HashMap::new();
    config.insert(
        "node_a_urat".to_string(),
        serde_yaml::Value::String("uart2".to_string()),
    );
    let error = world_error(quiet_can_environment(interconnect(
        "uart_cross_link",
        &["tester", "ecu"],
        config,
    )));

    assert!(
        error.contains("interconnects[0].config.node_a_urat is not supported for uart_cross_link"),
        "{error}"
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
