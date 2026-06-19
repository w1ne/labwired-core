// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Integration gate: `World::from_manifest` wires a `can_bus` interconnect.
//!
//! Tests:
//! 1. `from_manifest_unknown_can_node_errors` — ghost node returns a clear Err.
//! 2. `from_manifest_two_can_nodes_frame_crosses` — two H563 UDS-ECU nodes on a
//!    shared CanBus; a `0x7E8` UDS response appears in node "b"'s RX FIFO0,
//!    proving `from_manifest` correctly wired the `can_bus` interconnect.
//!
//! SKIP cleanly if the pre-built ELF is absent.
//! Run: cargo test -p labwired-core --test fdcan_multinode -- --nocapture

use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
use labwired_core::world::World;
use std::collections::HashMap;
use std::path::PathBuf;

fn ecu_root() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/h563-uds-ecu"
    ))
}

// ---- FDCAN1 memory-map constants (CMSIS stm32h563xx.h / stm32h563.yaml) -----
/// FDCAN1 peripheral base address on H563.
const FDCAN1_BASE: u64 = 0x4000_A400;
/// RXF0S register offset (fill level + pointers).
const REG_RXF0S: u64 = 0x090;
/// SRAMCAN start within the peripheral window.
const RAM_BASE: u64 = 0x800;
/// RX FIFO0 elements start at SRAMCAN + 0xB0; each element is 18 words (72 bytes).
const RF0_BASE: u64 = 0xB0;
const RF0_ELEMENT_BYTES: u64 = 18 * 4; // 72 bytes per element

/// Read a little-endian u32 from a machine via four `read_u8` calls.
fn read_u32_machine(machine: &dyn labwired_core::world::MachineTrait, addr: u64) -> u32 {
    let b0 = machine.read_u8(addr).unwrap_or(0) as u32;
    let b1 = machine.read_u8(addr + 1).unwrap_or(0) as u32;
    let b2 = machine.read_u8(addr + 2).unwrap_or(0) as u32;
    let b3 = machine.read_u8(addr + 3).unwrap_or(0) as u32;
    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

// ---- test 1: unknown ghost node yields a clear error -------------------------

#[test]
fn from_manifest_unknown_can_node_errors() {
    let env = EnvironmentManifest {
        schema_version: "1".into(),
        name: "bad".into(),
        nodes: vec![],
        interconnects: vec![InterconnectConfig {
            r#type: "can_bus".into(),
            nodes: vec!["ghost".into()],
            config: HashMap::new(),
        }],
    };
    match World::from_manifest(env, &ecu_root()) {
        Ok(_) => panic!("expected Err for ghost node, got Ok"),
        Err(e) => {
            let msg = e.to_string();
            assert!(msg.contains("ghost"), "must name the unknown node, got: {msg}");
        }
    }
}

// ---- test 2: two-node manifest — frame crosses A→B via CanBus ---------------

#[test]
fn from_manifest_two_can_nodes_frame_crosses() {
    let root = ecu_root();
    let elf_path = root.join("firmware/build/h563_uds_ecu.elf");
    if !elf_path.exists() {
        eprintln!(
            "SKIP fdcan_multinode: ELF not found at {elf_path:?}. \
             Build with: make -C examples/h563-uds-ecu/firmware"
        );
        return;
    }

    // Both nodes share the same system + firmware (h563-uds-ecu). Each node
    // has its own CAN diagnostic tester that injects a UDS request on 0x7E0
    // into the node's local FDCAN RX. The firmware processes the request and
    // transmits a response on 0x7E8. The `can_bus` interconnect forwards that
    // TX frame to every other node on the bus.
    let env = EnvironmentManifest {
        schema_version: "1".into(),
        name: "fdcan-twonode".into(),
        nodes: vec![
            NodeConfig {
                id: "a".into(),
                system: "system.yaml".into(),
                firmware: "firmware/build/h563_uds_ecu.elf".into(),
                config_overrides: HashMap::new(),
            },
            NodeConfig {
                id: "b".into(),
                system: "system.yaml".into(),
                firmware: "firmware/build/h563_uds_ecu.elf".into(),
                config_overrides: HashMap::new(),
            },
        ],
        interconnects: vec![InterconnectConfig {
            r#type: "can_bus".into(),
            nodes: vec!["a".into(), "b".into()],
            config: HashMap::new(),
        }],
    };

    let mut world = World::from_manifest(env, &root).expect("from_manifest must succeed");
    assert_eq!(world.machines.len(), 2, "two nodes expected");

    // Drive the simulation up to 200_000 iterations. After each step check
    // node "b"'s FDCAN1 RX FIFO0 fill level (RXF0S[6:0]). Once non-zero read
    // the head element (at F0GI) and extract the standard 11-bit id.
    let rxf0s_addr = FDCAN1_BASE + REG_RXF0S;

    let mut received_id: Option<u32> = None;
    let mut cross_iter: u64 = 0;

    for iter in 0..200_000u64 {
        world.step_all();

        if received_id.is_none() {
            let rxf0s = read_u32_machine(
                world.machines.get("b").unwrap().as_ref(),
                rxf0s_addr,
            );
            let fill = rxf0s & 0x7F;
            if fill > 0 {
                // F0GI (get-index): bits [13:8] — points to the oldest
                // (head) element the software can read next.
                let f0gi = (rxf0s >> 8) & 0x3F;
                // Address of element R0 word: SRAMCAN + RF0_BASE + F0GI * 72
                let r0_addr = FDCAN1_BASE
                    + RAM_BASE
                    + RF0_BASE
                    + (f0gi as u64) * RF0_ELEMENT_BYTES;
                let r0 = read_u32_machine(
                    world.machines.get("b").unwrap().as_ref(),
                    r0_addr,
                );
                let id = (r0 >> 18) & 0x7FF;
                if id != 0 {
                    eprintln!(
                        "node 'b' frame at iter {iter}: id=0x{id:03X} \
                         f0gi={f0gi} rxf0s=0x{rxf0s:08X}"
                    );
                }
                // Wait specifically for the UDS response id 0x7E8 — it can
                // only arrive via the CanBus from node 'a' (or node 'b'
                // receiving its own TX echoed back by the shared bus).
                if id == 0x7E8 {
                    received_id = Some(id);
                    cross_iter = iter;
                    break;
                }
            }
        }
    }

    let id = received_id.unwrap_or_else(|| {
        panic!(
            "node 'b' never received any CAN frame after 200_000 iterations; \
             from_manifest may have failed to wire the can_bus interconnect"
        )
    });

    eprintln!(
        "frame crossed at iter {cross_iter}: node 'b' RX FIFO0 id=0x{id:03X} \
         (expected 0x7E8)"
    );

    assert_eq!(
        id, 0x7E8,
        "expected UDS response id 0x7E8 in node 'b' RX FIFO0, got 0x{id:03X}"
    );
}
