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

const DEVICE_FW: &str = "../al2205-iolink-dido/firmware/al2205_dido.elf";

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
