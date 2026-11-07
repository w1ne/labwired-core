// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ch32fun's `examples/i2c_slave`, unmodified, on a CH32V003 answering an
//! STM32F103 controller over two open-drain world `gpio_net`s
//! (`examples/ch32v003-evt/i2c-slave`). The expected bytes come from the
//! controller source (`src/f1_controller.c`) and ch32fun's `i2c_slave.h`.

use labwired_config::EnvironmentManifest;
use labwired_core::world::World;
use std::path::PathBuf;

fn build(env_file: &str) -> World {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/ch32v003-evt/i2c-slave");
    let yaml = std::fs::read_to_string(dir.join(env_file)).unwrap();
    let manifest: EnvironmentManifest = serde_yaml::from_str(&yaml).unwrap();
    World::from_manifest(manifest, &dir).expect("world")
}

fn run_us(world: &mut World, us: u64) {
    let end = us * 1_000_000;
    let mut calls = 0u64;
    while world.round_now_ps().unwrap() < end {
        for (id, r) in world.step_all() {
            r.unwrap_or_else(|e| panic!("node {id}: {e:?}"));
        }
        calls += 1;
        assert!(calls < 50_000_000, "runaway");
    }
}

fn read(world: &World, node: &str, addr: u32, len: usize) -> Vec<u8> {
    world.machines[node].read_memory(addr, len).unwrap()
}

fn words(world: &World, node: &str, addr: u32, n: usize) -> Vec<u32> {
    read(world, node, addr, n * 4)
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

const GPIOA_OUTDR: u32 = 0x4001_080C;

#[test]
fn an_f103_controller_writes_and_reads_the_ch32fun_slave() {
    let mut world = build("env.yaml");
    run_us(&mut world, 20_000);
    let r = words(&world, "master", 0x2000_0100, 8);
    assert_eq!(
        &r[0..4],
        &[0x01, 0xAB, 0x5A, 0x00],
        "4-byte read from offset 0: {r:x?}"
    );
    // The byte the slave loaded into DR while the 4-byte read was NACKed.
    assert_eq!(r[4], 0x00, "single read: {r:x?}");
    assert_eq!(r[6], 1, "0x50 is NACKed: {r:x?}");
    assert_eq!(r[7], 1, "controller done: {r:x?}");
    assert_eq!(read(&world, "slave", 0x2000_0000, 3), [0x01, 0xAB, 0x5A]);
    assert_eq!(
        words(&world, "slave", GPIOA_OUTDR, 1)[0] & 0x4,
        0x4,
        "PA2 high"
    );
}

#[test]
fn a_write_past_the_register_array_wedges_the_bus() {
    let mut world = build("env-past-end.yaml");
    run_us(&mut world, 20_000);
    let r = words(&world, "master", 0x2000_0100, 8);
    assert_eq!(r[7], 0, "controller must still be stuck: {r:x?}");
    assert_eq!(read(&world, "slave", 0x2000_0000, 3), [0x00, 0xAB, 0x5A]);
    // i2c_slave_state.offset: the stale 0x77 left in DR.
    assert_eq!(read(&world, "slave", 0x2000_0005, 1), [0x77]);
    let scl = world
        .gpio_net_reports()
        .into_iter()
        .find(|n| n.name == "scl")
        .expect("scl net");
    assert!(!scl.level, "the slave holds SCL low");
}
