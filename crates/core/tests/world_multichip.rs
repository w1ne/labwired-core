// IO-Link multi-chip station integration tests.
//
// Task 3: World::from_manifest builds N Cortex-M nodes from an EnvironmentManifest
// and wires uart_cross_link interconnects. (The full master↔sensor PD-exchange
// proof is added in Task 5 once the master firmware exists.)

use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
use labwired_core::world::World;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn station_root() -> PathBuf {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/iolink-station"
    ))
    .to_path_buf()
}

const DEVICE_FW: &str = "../iolink-dido/firmware/iolink_dido.elf";

#[test]
fn from_manifest_builds_two_cortexm_nodes_and_uart_link() {
    // Two device-FW nodes wired uart2<->uart2. This exercises node construction,
    // ELF load + reset, the UART-link wiring, and lockstep stepping — without
    // needing the master firmware (Task 4).
    let env = EnvironmentManifest {
        schema_version: "1".into(),
        name: "twonode".into(),
        nodes: vec![
            NodeConfig {
                id: "n1".into(),
                system: "sensor/system.yaml".into(),
                firmware: DEVICE_FW.into(),
                config_overrides: HashMap::new(),
            },
            NodeConfig {
                id: "n2".into(),
                system: "sensor/system.yaml".into(),
                firmware: DEVICE_FW.into(),
                config_overrides: HashMap::new(),
            },
        ],
        interconnects: vec![InterconnectConfig {
            r#type: "uart_cross_link".into(),
            nodes: vec!["n1".into(), "n2".into()],
            config: HashMap::new(),
        }],
    };

    let mut world = World::from_manifest(env, &station_root()).expect("build world from manifest");
    assert_eq!(world.machines.len(), 2, "two nodes expected");

    for _ in 0..2000 {
        let results = world.step_all();
        assert!(
            results.values().all(|r| r.is_ok()),
            "a node failed to step: {results:?}"
        );
    }
}

// Task 5: the Phase-1 proof — a master chip running real iolinki-master firmware
// drives a real iolinki DEVICE-firmware sensor chip over the UartCrossLink and
// reaches OPERATE. Requires the built ELFs (master-fw/master.elf and the iolink-dido
// device ELF); skipped with a clear message if they are missing.
#[test]
fn master_chip_reaches_operate_with_real_sensor_chip() {
    let root = station_root();
    let master_elf = root.join("master-fw/master.elf");
    let device_elf = root.join("../iolink-dido/firmware/iolink_dido.elf");
    if !master_elf.exists() || !device_elf.exists() {
        eprintln!(
            "SKIP: build ELFs first (make -C examples/iolink-station/master-fw && \
             make -C examples/iolink-dido/firmware)"
        );
        return;
    }

    let env = EnvironmentManifest::from_file(root.join("env.yaml")).expect("parse env.yaml");
    let mut world = World::from_manifest(env, &root).expect("build station world");

    // g_master_state lives at 0x20000000 (D g_master_state in the master map);
    // 3 == IOLINK_MASTER_STATE_OPERATE.
    const STATE_ADDR: u64 = 0x2000_0000;
    const PD0_ADDR: u64 = 0x2000_0001;
    const OPERATE: u8 = 3;

    // The sensor publishes its 74HC165 input byte as process data; the
    // sensor/system.yaml presets `inputs: 165` (0xA5). 0xFF is the master's
    // pre-exchange sentinel, so a real PD read must land on 0xA5.
    const EXPECTED_PD: u8 = 0xA5;

    let mut reached_operate = false;
    let mut last_state = 0u8;
    let mut pd0 = 0xFFu8;
    for _ in 0..5_000_000u64 {
        world.step_all();
        let master = world.machines.get("master").unwrap();
        last_state = master.read_u8(STATE_ADDR).unwrap();
        if last_state == OPERATE {
            reached_operate = true;
        }
        pd0 = master.read_u8(PD0_ADDR).unwrap();
        // Stop once we have proof of a real cyclic PD exchange in OPERATE.
        if reached_operate && pd0 != 0xFF {
            break;
        }
    }

    assert!(
        reached_operate,
        "master chip never reached OPERATE driving the real sensor chip; last_state={last_state:#x}"
    );
    assert_eq!(
        pd0, EXPECTED_PD,
        "master must read the sensor's real published process data (0x{EXPECTED_PD:02x}), got {pd0:#x}"
    );
    eprintln!("master reached OPERATE and exchanged real PD = {pd0:#x} with the sensor chip");
}

// Task 6: 4-port station. One master chip runs a 4-port iolinki-master
// controller; each port is wired (USART2/3/4/5) to its own sensor chip running
// the real device firmware, each preset to a distinct palindrome PD byte. All
// four ports must reach OPERATE and read their own sensor's exact PD — proving
// four independent, real IO-Link links with no cross-talk.
#[test]
fn four_port_station_all_sensors_operate_with_distinct_pd() {
    let root = station_root();
    let master_elf = root.join("master-fw-4port/master.elf");
    let device_elf = root.join("../iolink-dido/firmware/iolink_dido.elf");
    if !master_elf.exists() || !device_elf.exists() {
        eprintln!(
            "SKIP: build ELFs first (make -C examples/iolink-station/master-fw-4port && \
             make -C examples/iolink-dido/firmware)"
        );
        return;
    }

    let env = EnvironmentManifest::from_file(root.join("env4.yaml")).expect("parse env4.yaml");
    let mut world = World::from_manifest(env, &root).expect("build 4-port station");

    const STATE: u64 = 0x2000_0000; // g_master_state[4]
    const PD: u64 = 0x2000_0004; // g_master_pd[4]
    const OPERATE: u8 = 3;
    // sensor1..4 input presets (palindrome bytes), bit-order-invariant.
    let expected: [u8; 4] = [0xA5, 0x3C, 0xC3, 0x5A];

    let mut done = false;
    for _ in 0..40_000_000u64 {
        world.step_all();
        let m = world.machines.get("master").unwrap();
        let all = (0..4u64).all(|i| {
            m.read_u8(STATE + i).unwrap() == OPERATE
                && m.read_u8(PD + i).unwrap() == expected[i as usize]
        });
        if all {
            done = true;
            break;
        }
    }

    let m = world.machines.get("master").unwrap();
    let states: Vec<u8> = (0..4).map(|i| m.read_u8(STATE + i).unwrap()).collect();
    let pds: Vec<u8> = (0..4).map(|i| m.read_u8(PD + i).unwrap()).collect();
    assert!(
        done,
        "not all 4 ports reached OPERATE with their expected PD; \
         states={states:02x?} pds={pds:02x?} expected={expected:02x?}"
    );
    eprintln!("4-port station: all ports OPERATE; PDs={pds:02x?}");
}
