//! Throughput of a three-node STM32F103 world on a timed `can_bus`, against
//! the same three nodes with no bus at all.
//!
//! The playground's CAN demo needs a three-node bus to run at least real
//! time natively: a dashboard follows a throttle slider live. This probe
//! reports wall time per simulated millisecond for
//!
//! - `no can_bus`: the three machines advanced to the same cycle count with
//!   nothing between them, at the world's default tick interval 1 and at the
//!   widest safe interval a timed-CAN node gets (the floor),
//! - `can_bus idle`: the bus configured on every node, nothing sent,
//! - `can_bus traffic`: an engine frame every millisecond, received and
//!   released by a dashboard, and a sensor on the wrong bit rate that goes
//!   error-passive and bus-off.
//!
//! `cargo test --release -p labwired-core --test world_can_perf -- --ignored
//! --nocapture` prints the numbers.

use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{MachineTrait, ResolvedWorldNode, World};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const RCC_CFGR: u64 = 0x4002_1004;
const RCC_APB1ENR: u64 = 0x4002_101C;
const CAN: u64 = 0x4000_6400;
const MCR: u64 = CAN;
const TSR: u64 = CAN + 0x08;
const RF0R: u64 = CAN + 0x0C;
const BTR: u64 = CAN + 0x1C;
const TI0R: u64 = CAN + 0x180;
const FMR: u64 = CAN + 0x200;
const FS1R: u64 = CAN + 0x20C;
const FA1R: u64 = CAN + 0x21C;
const F0R1: u64 = CAN + 0x240;
const BTR_500K: u32 = (2 << 20) | (13 << 16) | 3;
const BTR_250K: u32 = (2 << 20) | (13 << 16) | 7;
const HZ: u64 = 72_000_000;
const MS: u64 = 1_000_000_000;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A minimal STM32F103 firmware ELF: the vector table, then a loop that
/// polls bxCAN RF0R and accumulates, as a CAN node's main loop does. Every
/// iteration is a peripheral load through the bus, so the loop neither
/// sleeps (no idle fast-forward) nor runs on the RAM fast path: each
/// simulated cycle is an executed instruction.
///
/// ```text
/// 0x08000000  .word 0x20002000, 0x08000009   ; SP, reset
/// 0x08000008  ldr  r0, [pc, #8]              ; r0 = &RF0R
/// 0x0800000A  ldr  r1, [r0]                  ; poll
/// 0x0800000C  adds r2, r2, r1
/// 0x0800000E  adds r3, #1
/// 0x08000010  b    0x0800000A
/// 0x08000012  nop
/// 0x08000014  .word 0x4000640C
/// ```
fn polling_firmware() -> Vec<u8> {
    let mut image = Vec::new();
    image.extend_from_slice(&0x2000_2000u32.to_le_bytes());
    image.extend_from_slice(&0x0800_0009u32.to_le_bytes());
    for half in [0x4802u16, 0x6801, 0x1852, 0x3301, 0xE7FB, 0xBF00] {
        image.extend_from_slice(&half.to_le_bytes());
    }
    image.extend_from_slice(&(RF0R as u32).to_le_bytes());
    let mut elf = vec![0u8; 0x100];
    elf[..16].copy_from_slice(&[0x7F, b'E', b'L', b'F', 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let put16 =
        |e: &mut Vec<u8>, at: usize, v: u16| e[at..at + 2].copy_from_slice(&v.to_le_bytes());
    let put32 =
        |e: &mut Vec<u8>, at: usize, v: u32| e[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put16(&mut elf, 16, 2); // ET_EXEC
    put16(&mut elf, 18, 40); // EM_ARM
    put32(&mut elf, 20, 1);
    put32(&mut elf, 24, 0x0800_0009); // entry
    put32(&mut elf, 28, 52); // phoff
    put32(&mut elf, 36, 0x0500_0200); // EABI5
    put16(&mut elf, 40, 52);
    put16(&mut elf, 42, 32);
    put16(&mut elf, 44, 1);
    put16(&mut elf, 46, 40);
    // PT_LOAD of the image at 0x08000000.
    put32(&mut elf, 52, 1);
    put32(&mut elf, 56, 0x100);
    put32(&mut elf, 60, 0x0800_0000);
    put32(&mut elf, 64, 0x0800_0000);
    put32(&mut elf, 68, image.len() as u32);
    put32(&mut elf, 72, image.len() as u32);
    put32(&mut elf, 76, 5);
    put32(&mut elf, 80, 4);
    elf.extend_from_slice(&image);
    elf
}

const IDS: [&str; 3] = ["dash", "engine", "sensor"];

fn world(with_can: bool) -> World {
    let interconnects = if with_can {
        vec![InterconnectConfig {
            r#type: "can_bus".to_string(),
            nodes: IDS.iter().map(|id| id.to_string()).collect(),
            config: HashMap::from([(
                "peripheral".to_string(),
                serde_yaml::Value::String("bxcan1".to_string()),
            )]),
        }]
    } else {
        Vec::new()
    };
    let manifest = EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "can-perf".to_string(),
        nodes: IDS
            .iter()
            .map(|id| NodeConfig {
                id: id.to_string(),
                system: "unused.yaml".to_string(),
                firmware: "unused.elf".to_string(),
                config_overrides: HashMap::new(),
                profile: None,
            })
            .collect(),
        interconnects,
        rf: None,
    };
    let system_path = repo_root().join("crates/core/tests/fixtures/f103-can-world-system.yaml");
    let system = labwired_config::SystemManifest::from_file(&system_path).unwrap();
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&system.chip),
    )
    .unwrap();
    let nodes = IDS
        .iter()
        .map(|id| ResolvedWorldNode {
            id: id.to_string(),
            system: system.clone(),
            chip: chip.clone(),
            firmware: NodeFirmware::Elf(polling_firmware()),
            blobs: Default::default(),
        })
        .collect();
    World::from_resolved(manifest, nodes).unwrap()
}

/// Every node is in its poll loop (not faulted) and has run it.
fn assert_polling(world: &World) {
    for id in IDS {
        let m = &world.machines[id];
        assert!(
            (0x0800_000A..0x0800_0012).contains(&m.get_pc()),
            "{id} pc {:#x}",
            m.get_pc()
        );
        assert!(m.get_register(3) > 1000, "{id} loop count");
    }
}

fn wr(m: &mut dyn MachineTrait, addr: u64, value: u32) {
    for byte in (0..4).rev() {
        m.write_u8(addr + byte, (value >> (byte * 8)) as u8)
            .unwrap();
    }
}

fn rd(m: &dyn MachineTrait, addr: u64) -> u32 {
    u32::from_le_bytes(std::array::from_fn(|b| m.read_u8(addr + b as u64).unwrap()))
}

fn start_bxcan(m: &mut dyn MachineTrait, btr: u32) {
    let cfgr = rd(m, RCC_CFGR) & !(7 << 8) | (4 << 8);
    wr(m, RCC_CFGR, cfgr);
    let enr = rd(m, RCC_APB1ENR) | (1 << 25);
    wr(m, RCC_APB1ENR, enr);
    wr(m, MCR, 1);
    wr(m, BTR, btr);
    wr(m, FMR, 1);
    wr(m, FS1R, 1);
    wr(m, F0R1, 0);
    wr(m, F0R1 + 4, 0);
    wr(m, FA1R, 1);
    wr(m, FMR, 0);
    wr(m, MCR, 0);
}

fn send(m: &mut dyn MachineTrait, id: u32, data: u32) {
    wr(m, TI0R + 8, data);
    wr(m, TI0R + 4, 2);
    wr(m, TI0R, (id << 21) | 1);
}

/// Wall µs per simulated ms over `sim_ms`, after 1 ms of warm-up.
fn time_world(world: &mut World, sim_ms: u64, mut firmware: impl FnMut(&mut World)) -> f64 {
    let step = MS / 5;
    let mut t = MS;
    world.run_until_ps(t).unwrap();
    let started = Instant::now();
    let end = t + sim_ms * MS;
    while t < end {
        t += step;
        let results = world.run_until_ps(t).unwrap();
        assert!(results.values().all(Result::is_ok));
        firmware(world);
    }
    let us = started.elapsed().as_secs_f64() * 1e6 / sim_ms as f64;
    assert_polling(world);
    us
}

fn no_can_floor(sim_ms: u64, widest: bool) -> f64 {
    let mut world = world(false);
    if widest {
        for id in IDS {
            world.machines.get_mut(id).unwrap().use_widest_safe_tick();
        }
    }
    let mut cycles = HZ / 1000;
    for id in IDS {
        world
            .machines
            .get_mut(id)
            .unwrap()
            .advance_to_cycle(cycles)
            .unwrap();
    }
    let started = Instant::now();
    let end = cycles + sim_ms * HZ / 1000;
    while cycles < end {
        cycles += HZ / 5000;
        for id in IDS {
            world
                .machines
                .get_mut(id)
                .unwrap()
                .advance_to_cycle(cycles)
                .unwrap();
        }
    }
    let us = started.elapsed().as_secs_f64() * 1e6 / sim_ms as f64;
    assert_polling(&world);
    us
}

fn can_idle(sim_ms: u64) -> f64 {
    let mut world = world(true);
    for id in IDS {
        start_bxcan(world.machines.get_mut(id).unwrap().as_mut(), BTR_500K);
    }
    time_world(&mut world, sim_ms, |_| {})
}

fn can_traffic(sim_ms: u64) -> f64 {
    let mut world = world(true);
    start_bxcan(world.machines.get_mut("dash").unwrap().as_mut(), BTR_500K);
    start_bxcan(world.machines.get_mut("engine").unwrap().as_mut(), BTR_500K);
    start_bxcan(world.machines.get_mut("sensor").unwrap().as_mut(), BTR_250K);
    send(
        world.machines.get_mut("sensor").unwrap().as_mut(),
        0x200,
        42,
    );
    let mut ms = 0u64;
    let mut tick = 0;
    let us = time_world(&mut world, sim_ms, |w| {
        tick += 1;
        let engine = w.machines.get_mut("engine").unwrap().as_mut();
        if tick % 5 == 0 && rd(engine, TSR) & (1 << 26) != 0 {
            ms += 1;
            send(engine, 0x100, ms as u32);
        }
        let dash = w.machines.get_mut("dash").unwrap().as_mut();
        while rd(dash, RF0R) & 3 != 0 {
            wr(dash, RF0R, 1 << 5);
        }
    });
    let bus = &world.can_bus_reports()[0];
    assert!(bus.totals.ok > sim_ms / 2, "{:?}", bus.totals);
    assert!(bus.totals.error > 0);
    us
}

#[test]
#[ignore = "manual perf probe; run with --release -- --ignored --nocapture"]
fn three_f103_can_world_throughput() {
    const SIM_MS: u64 = 100;
    let floor1 = no_can_floor(SIM_MS, false);
    let floor = no_can_floor(SIM_MS, true);
    let idle = can_idle(SIM_MS);
    let traffic = can_traffic(SIM_MS);
    println!("\n=== 3x STM32F103 world, wall µs per simulated ms (real time = 1000) ===");
    println!("no can_bus, tick interval 1 (world default) : {floor1:9.1}");
    println!("no can_bus, widest safe tick                : {floor:9.1}");
    println!(
        "can_bus idle                                : {idle:9.1}  ({:.2}x)",
        idle / floor
    );
    println!(
        "can_bus traffic                             : {traffic:9.1}  ({:.2}x)\n",
        traffic / floor
    );
}

/// The structural half of the fix, pinned without timing: putting bxCAN on
/// a timed bus does not narrow the widest safe tick batch of its node. Under
/// the event scheduler the F103 bus is walk-deleted and stays so; a
/// controller that put the walk back would pin the whole node to one
/// instruction per batch (the 2026-10-11 state, ~10x slower in this world).
/// Without the scheduler the interval is 1 either way.
#[test]
fn a_timed_bxcan_keeps_the_walk_deleted() {
    let system_path = repo_root().join("crates/core/tests/fixtures/f103-can-world-system.yaml");
    let system = labwired_config::SystemManifest::from_file(&system_path).unwrap();
    let chip = labwired_config::ChipDescriptor::from_file(
        system_path.parent().unwrap().join(&system.chip),
    )
    .unwrap();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &system).unwrap();
    let before = bus.max_safe_tick_interval();
    bus.attach_timed_can_by_id("bxcan1").unwrap();
    assert_eq!(bus.max_safe_tick_interval(), before);
}

/// The timed bus costs little over the nodes themselves: a three-node world
/// with CAN traffic and a wrong-bit-rate node within 2x of the same nodes
/// with no bus (measured 1.07x idle, 1.13x with traffic on 2026-10-11; it
/// was 3.8x before the controllers left the walk; 1.3x in a build without
/// the event scheduler, where both pay the walk). A ratio, so it holds on any
/// machine speed.
#[cfg(not(debug_assertions))]
#[test]
fn timed_can_overhead_stays_small() {
    const SIM_MS: u64 = 30;
    // Best of three, so one descheduled run on a busy CI host does not fail it.
    let best = |f: &dyn Fn() -> f64| (0..3).map(|_| f()).fold(f64::MAX, f64::min);
    let floor = best(&|| no_can_floor(SIM_MS, true));
    let traffic = best(&|| can_traffic(SIM_MS));
    assert!(
        traffic <= 2.0 * floor,
        "timed CAN world {traffic:.0} µs per simulated ms vs {floor:.0} without the bus"
    );
}
