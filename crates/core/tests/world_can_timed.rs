//! A world `can_bus` between real STM32F103 nodes is timed and
//! fault-confined: frames take their wire time, a node on the wrong bit rate
//! goes error-passive and then bus-off, and the bxCAN registers firmware
//! polls (ESR, TSR, RF0R) show it.
//!
//! The nodes run a GPIO bench firmware that never touches bxCAN; every CAN
//! register access below goes through World -> MachineTrait -> SystemBus, as
//! firmware's would.

use labwired_config::{EnvironmentManifest, InterconnectConfig, NodeConfig};
use labwired_core::bus::bus_trace::{BusDir, BusPayload};
use labwired_core::network::timed_can::{CanErrorState, CanTxOutcome};
use labwired_core::world::{MachineTrait, World};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const RCC_BASE: u64 = 0x4002_1000;
const RCC_CFGR: u64 = RCC_BASE + 0x04;
const RCC_APB1ENR: u64 = RCC_BASE + 0x1C;
const CAN: u64 = 0x4000_6400;
const MCR: u64 = CAN;
const TSR: u64 = CAN + 0x08;
const RF0R: u64 = CAN + 0x0C;
const ESR: u64 = CAN + 0x18;
const BTR: u64 = CAN + 0x1C;
const TI0R: u64 = CAN + 0x180;
const RI0R: u64 = CAN + 0x1B0;
const FMR: u64 = CAN + 0x200;
const FS1R: u64 = CAN + 0x20C;
const FA1R: u64 = CAN + 0x21C;
const F0R1: u64 = CAN + 0x240;

const MCR_INRQ: u32 = 1;
const MCR_ABOM: u32 = 1 << 6;
/// 500 kbit/s from PCLK1 = 36 MHz: BRP 3, TS1 13, TS2 2 (18 tq).
const BTR_500K: u32 = (2 << 20) | (13 << 16) | 3;
/// The field bug: BRP 7 gives 250 kbit/s.
const BTR_250K: u32 = (2 << 20) | (13 << 16) | 7;

const US: u64 = 1_000_000;
const MS: u64 = 1_000_000_000;
const BIT: u64 = 2 * US;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node(id: &str) -> NodeConfig {
    NodeConfig {
        id: id.to_string(),
        system: "crates/core/tests/fixtures/f103-can-world-system.yaml".to_string(),
        firmware: "examples/f103-fidelity-bench/firmware/build/gpiobug.elf".to_string(),
        config_overrides: HashMap::new(),
        profile: None,
    }
}

fn can_world(order: &[&str], bitrate: Option<u64>) -> World {
    let mut config = HashMap::from([(
        "peripheral".to_string(),
        serde_yaml::Value::String("bxcan1".to_string()),
    )]);
    if let Some(b) = bitrate {
        config.insert("bitrate".to_string(), serde_yaml::Value::Number(b.into()));
    }
    let manifest = EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "three-f103-on-can".to_string(),
        nodes: order.iter().map(|id| node(id)).collect(),
        interconnects: vec![InterconnectConfig {
            r#type: "can_bus".to_string(),
            nodes: order.iter().map(|id| id.to_string()).collect(),
            config,
        }],
        rf: None,
    };
    World::from_manifest(manifest, &repo_root()).expect("three F103 nodes on one bus")
}

/// Word write as a byte sequence, high byte first: the bxCAN byte path is a
/// read-modify-write of the word, so TXRQ (bit 0) must land last.
fn wr(m: &mut dyn MachineTrait, addr: u64, value: u32) {
    for byte in (0..4).rev() {
        m.write_u8(addr + byte, (value >> (byte * 8)) as u8)
            .unwrap();
    }
}

fn rd(m: &dyn MachineTrait, addr: u64) -> u32 {
    u32::from_le_bytes(std::array::from_fn(|b| m.read_u8(addr + b as u64).unwrap()))
}

fn machine<'a>(world: &'a mut World, id: &str) -> &'a mut dyn MachineTrait {
    world.machines.get_mut(id).unwrap().as_mut()
}

/// What firmware's MX_CAN_Init does: CAN1 clock on, APB1 = HCLK / 2 =
/// 36 MHz, init mode, bit timing, an accept-all filter, normal mode.
fn start_bxcan(m: &mut dyn MachineTrait, btr: u32, mcr: u32) {
    let cfgr = rd(m, RCC_CFGR) & !(7 << 8) | (4 << 8);
    wr(m, RCC_CFGR, cfgr);
    let apb1enr = rd(m, RCC_APB1ENR) | (1 << 25);
    wr(m, RCC_APB1ENR, apb1enr);
    wr(m, MCR, MCR_INRQ | mcr);
    wr(m, BTR, btr);
    wr(m, FMR, 1);
    wr(m, FS1R, 1);
    wr(m, F0R1, 0);
    wr(m, F0R1 + 4, 0);
    wr(m, FA1R, 1);
    wr(m, FMR, 0);
    wr(m, MCR, mcr);
}

fn send(m: &mut dyn MachineTrait, id: u32, data: &[u8]) {
    let mut lo = [0u8; 4];
    let mut hi = [0u8; 4];
    for (i, b) in data.iter().enumerate() {
        if i < 4 {
            lo[i] = *b;
        } else {
            hi[i - 4] = *b;
        }
    }
    wr(m, TI0R + 8, u32::from_le_bytes(lo));
    wr(m, TI0R + 12, u32::from_le_bytes(hi));
    wr(m, TI0R + 4, data.len() as u32);
    wr(m, TI0R, (id << 21) | 1);
}

/// Dashboard firmware: read and release every message in RX FIFO0.
fn drain_fifo0(m: &mut dyn MachineTrait, seen: &mut Vec<u32>) {
    while rd(m, RF0R) & 3 != 0 {
        seen.push(rd(m, RI0R) >> 21);
        wr(m, RF0R, 1 << 5); // RFOM0
    }
}

fn rx_ids(world: &World, id: &str) -> Vec<u32> {
    world.machines[id]
        .bus_trace_snapshot()
        .into_iter()
        .filter_map(|e| match e.payload {
            BusPayload::Can {
                direction: BusDir::Rx,
                id,
                ..
            } => Some(id),
            _ => None,
        })
        .collect()
}

fn run(world: &mut World, until_ps: u64) {
    let results = world
        .run_until_ps(until_ps)
        .expect("a CAN world has a round clock");
    assert!(results.values().all(Result::is_ok), "{results:?}");
}

#[test]
fn a_frame_between_f103_nodes_takes_its_wire_time_and_the_registers_show_it() {
    let mut world = can_world(&["dash", "engine", "sensor"], None);
    for id in ["dash", "engine", "sensor"] {
        start_bxcan(machine(&mut world, id), BTR_500K, 0);
    }
    run(&mut world, 100 * US);
    let t0 = world.round_now_ps().unwrap();
    send(machine(&mut world, "engine"), 0x100, &[0x0F, 0xA0]);
    // RM0008 §24.9.2: while the frame is pending TME0 is clear and no
    // RQCP0/TXOK0 is set.
    let tsr = rd(machine(&mut world, "engine"), TSR);
    assert_eq!(tsr & (1 << 26), 0, "TME0 clear while pending");
    assert_eq!(tsr & 0x3, 0);

    // 0x100 with 0f a0: 44 + 16 bits and 6 stuff bits = 66 bits = 132 µs.
    let frame_ps = 66 * BIT;
    run(&mut world, t0 + frame_ps - 10 * US);
    assert!(rx_ids(&world, "dash").is_empty(), "not before its EOF");
    assert_eq!(rd(machine(&mut world, "dash"), RF0R) & 3, 0);
    run(&mut world, t0 + frame_ps + 20 * US);
    assert_eq!(rx_ids(&world, "dash"), vec![0x100]);
    assert_eq!(rx_ids(&world, "sensor"), vec![0x100]);
    assert!(rx_ids(&world, "engine").is_empty(), "no echo to the sender");
    let dash = machine(&mut world, "dash");
    assert_eq!(rd(dash, RF0R) & 3, 1, "FMP0");
    assert_eq!(rd(dash, RI0R) >> 21, 0x100);
    let engine = machine(&mut world, "engine");
    assert_eq!(rd(engine, TSR) & 0x3, 0x3, "RQCP0 | TXOK0");
    assert_ne!(rd(engine, TSR) & (1 << 26), 0, "TME0");
    assert_eq!(rd(engine, ESR), 0);

    let reports = world.can_bus_reports();
    assert_eq!(reports.len(), 1);
    let bus = &reports[0];
    assert_eq!(
        (bus.id.as_str(), bus.bitrate, bus.timed),
        ("can0", 500_000, true)
    );
    assert_eq!(bus.frames.len(), 1);
    let f = &bus.frames[0];
    assert_eq!(
        (f.from.as_str(), f.id, f.dlc, f.data.as_str()),
        ("engine", 0x100, 2, "0fa0")
    );
    assert_eq!(f.result, CanTxOutcome::Ok);
    assert!(
        f.t_ps >= t0 && f.t_ps < t0 + 2 * US,
        "SOF at the request: {} vs {t0}",
        f.t_ps
    );
    assert_eq!(f.end_ps - f.t_ps, frame_ps);
    let rates: Vec<_> = bus
        .nodes
        .iter()
        .map(|n| (n.node.as_str(), n.bitrate))
        .collect();
    assert_eq!(
        rates,
        vec![
            ("dash", Some(500_000)),
            ("engine", Some(500_000)),
            ("sensor", Some(500_000))
        ]
    );
    assert!(bus
        .nodes
        .iter()
        .all(|n| n.clock_hz == 36_000_000 && n.on_bus));
}

#[test]
fn arbitration_between_f103_nodes_sends_the_lower_id_first() {
    let mut world = can_world(&["a", "b", "c"], None);
    for id in ["a", "b", "c"] {
        start_bxcan(machine(&mut world, id), BTR_500K, 0);
    }
    run(&mut world, 100 * US);
    send(machine(&mut world, "a"), 0x300, &[1]);
    send(machine(&mut world, "b"), 0x080, &[2]);
    run(&mut world, 2 * MS);
    assert_eq!(rx_ids(&world, "c"), vec![0x080, 0x300]);
    let results: Vec<_> = world.can_bus_reports()[0]
        .frames
        .iter()
        .map(|f| (f.from.clone(), f.result))
        .collect();
    assert_eq!(
        results,
        vec![
            ("b".to_string(), CanTxOutcome::Ok),
            ("a".to_string(), CanTxOutcome::LostArbitration),
            ("a".to_string(), CanTxOutcome::Ok)
        ]
    );
    // RM0008: ALST0 reports the lost attempt until the retry completes.
    assert_eq!(rd(machine(&mut world, "a"), TSR) & 0xF, 0x3);
}

#[test]
fn a_lone_node_gets_no_ack_and_stops_at_error_passive() {
    let mut world = can_world(&["a", "b", "c"], None);
    start_bxcan(machine(&mut world, "a"), BTR_500K, 0);
    run(&mut world, 100 * US);
    send(machine(&mut world, "a"), 0x123, &[]);
    run(&mut world, 20 * MS);
    let esr = rd(machine(&mut world, "a"), ESR);
    // TEC 128, error-passive, LEC = 011 acknowledgment error.
    assert_eq!((esr >> 16) & 0xFF, 128);
    assert_eq!(esr & 0b111, 0b011, "EPVF | EWGF, not bus-off");
    assert_eq!((esr >> 4) & 7, 3, "LEC ack error");
    assert_eq!(
        rd(machine(&mut world, "a"), TSR) & 0xF,
        0b1000,
        "TERR0, still pending"
    );
    let bus = &world.can_bus_reports()[0];
    assert!(bus.frames.iter().all(|f| f.result == CanTxOutcome::NoAck));
    let a = bus.nodes.iter().find(|n| n.node == "a").unwrap();
    assert_eq!((a.tec, a.state), (128, CanErrorState::Passive));
    let b = bus.nodes.iter().find(|n| n.node == "b").unwrap();
    assert!(!b.on_bus, "a controller still in reset is off the bus");
}

/// The promo story at register level: a new sensor with BRP one off joins.
#[test]
fn a_wrong_bitrate_f103_goes_passive_then_bus_off_then_recovers_and_joins_once_fixed() {
    let mut world = can_world(&["dash", "engine", "sensor"], None);
    start_bxcan(machine(&mut world, "dash"), BTR_500K, 0);
    start_bxcan(machine(&mut world, "engine"), BTR_500K, 0);
    start_bxcan(machine(&mut world, "sensor"), BTR_250K, 0);

    let mut sensor_esr = Vec::new();
    let mut t = 100 * US;
    let mut sent_temp = false;
    let mut dash_seen = Vec::new();
    while t < 60 * MS {
        run(&mut world, t);
        // Engine firmware: a new RPM frame whenever mailbox 0 is free.
        let engine = machine(&mut world, "engine");
        if rd(engine, TSR) & (1 << 26) != 0 {
            send(engine, 0x100, &[0x0F, 0xA0]);
        }
        if !sent_temp && t >= 5 * MS {
            send(machine(&mut world, "sensor"), 0x200, &[42]);
            sent_temp = true;
        }
        drain_fifo0(machine(&mut world, "dash"), &mut dash_seen);
        sensor_esr.push(rd(machine(&mut world, "sensor"), ESR));
        t += 200 * US;
    }
    // EPVF (bit 1) came on, then BOFF (bit 2): error-passive, then bus-off.
    let passive_at = sensor_esr
        .iter()
        .position(|e| e & 0b010 != 0)
        .expect("passive");
    let off_at = sensor_esr
        .iter()
        .position(|e| e & 0b100 != 0)
        .expect("bus-off");
    assert!(passive_at < off_at);
    let esr = rd(machine(&mut world, "sensor"), ESR);
    assert_eq!(esr & 0b111, 0b111, "BOFF | EPVF | EWGF");
    assert_eq!(
        (esr >> 16) & 0xFF,
        248,
        "TEC 248: the overflowing +8 is not stored"
    );
    // The healthy nodes stayed error-active, and the dashboard got the
    // engine's frames once the sensor stopped destroying them.
    for id in ["dash", "engine"] {
        assert_eq!(
            rd(machine(&mut world, id), ESR) & 0b110,
            0,
            "{id} error-active"
        );
    }
    let engine_frames = dash_seen.iter().filter(|i| **i == 0x100).count();
    assert!(engine_frames > 10, "{engine_frames}");
    assert!(!dash_seen.contains(&0x200));

    // Without ABOM the sensor stays bus-off until firmware cycles INRQ; then
    // it needs 128 x 11 recessive bits (2.816 ms of idle bus at 500 kbit/s).
    let now = world.round_now_ps().unwrap();
    run(&mut world, now + 5 * MS);
    assert_ne!(
        rd(machine(&mut world, "sensor"), ESR) & 0b100,
        0,
        "no ABOM: still off"
    );
    let sensor = machine(&mut world, "sensor");
    // Fix the one number while in init: BRP 7 -> 3.
    wr(sensor, MCR, MCR_INRQ | MCR_ABOM);
    wr(sensor, BTR, BTR_500K);
    wr(sensor, MCR, MCR_ABOM);
    let armed = world.round_now_ps().unwrap();
    run(&mut world, armed + 128 * 11 * BIT - 50 * US);
    assert_ne!(
        rd(machine(&mut world, "sensor"), ESR) & 0b100,
        0,
        "recovery is not instant"
    );
    run(&mut world, armed + 128 * 11 * BIT + 2 * MS);
    let esr = rd(machine(&mut world, "sensor"), ESR);
    assert_eq!(esr & 0b111, 0, "error-active again");
    assert_eq!(esr >> 16, 0, "TEC and REC reset by the recovery");
    // Its pending temperature frame now gets through.
    let now = world.round_now_ps().unwrap();
    drain_fifo0(machine(&mut world, "dash"), &mut dash_seen);
    run(&mut world, now + 5 * MS);
    drain_fifo0(machine(&mut world, "dash"), &mut dash_seen);
    assert!(dash_seen.contains(&0x200), "the fixed node joined");
    let bus = &world.can_bus_reports()[0];
    let sensor = bus.nodes.iter().find(|n| n.node == "sensor").unwrap();
    assert_eq!(
        (sensor.bitrate, sensor.state),
        (Some(500_000), CanErrorState::Active)
    );
    assert!(bus.totals.error > 0 && bus.totals.ok > 0);
}

#[test]
fn the_bus_bitrate_comes_from_the_manifest() {
    let mut world = can_world(&["a", "b", "c"], Some(250_000));
    start_bxcan(machine(&mut world, "a"), BTR_250K, 0);
    start_bxcan(machine(&mut world, "b"), BTR_250K, 0);
    run(&mut world, 100 * US);
    send(machine(&mut world, "a"), 0x100, &[0x0F, 0xA0]);
    run(&mut world, 2 * MS);
    assert_eq!(rx_ids(&world, "b"), vec![0x100]);
    let bus = &world.can_bus_reports()[0];
    assert_eq!(bus.bitrate, 250_000);
    assert_eq!(bus.frames[0].end_ps - bus.frames[0].t_ps, 66 * 2 * BIT);
}

#[test]
fn manifest_order_does_not_change_the_bus() {
    let story = |order: &[&str]| {
        let mut world = can_world(order, None);
        start_bxcan(machine(&mut world, "dash"), BTR_500K, 0);
        start_bxcan(machine(&mut world, "engine"), BTR_500K, 0);
        start_bxcan(machine(&mut world, "sensor"), BTR_250K, MCR_ABOM);
        run(&mut world, 100 * US);
        send(machine(&mut world, "engine"), 0x100, &[1, 2]);
        send(machine(&mut world, "sensor"), 0x200, &[3]);
        run(&mut world, 10 * MS);
        let esr: Vec<u32> = ["dash", "engine", "sensor"]
            .iter()
            .map(|id| rd(machine(&mut world, id), ESR))
            .collect();
        (world.can_bus_reports(), esr)
    };
    assert_eq!(
        story(&["dash", "engine", "sensor"]),
        story(&["sensor", "dash", "engine"])
    );
}

#[test]
fn a_world_without_can_keeps_its_stepping_path() {
    let manifest = EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "no-can".to_string(),
        nodes: vec![node("a"), node("b")],
        interconnects: vec![],
        rf: None,
    };
    let mut world = World::from_manifest(manifest, &repo_root()).unwrap();
    assert!(world.can_bus_reports().is_empty());
    assert_eq!(world.round_now_ps(), None, "no round clock: plain lockstep");
    world.step_all();
    assert_eq!(
        world.machines["a"].total_cycles(),
        world.machines["b"].total_cycles()
    );
}
