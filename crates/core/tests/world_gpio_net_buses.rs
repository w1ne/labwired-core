// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Hardware SPI and I²C between two STM32G071s over world `gpio_net`s, bit by
//! bit at the pads (`examples/gpio-net-buses`).
//!
//! Every expected byte is hand-derived from the firmware sources in
//! `examples/gpio-net-buses/src`, not measured.

use labwired_config::EnvironmentManifest;
use labwired_core::network::gpio_net::{GpioNetReport, GPIO_NET_CONTENTION, GPIO_NET_FLOATING};
use labwired_core::world::World;
use std::path::PathBuf;

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/gpio-net-buses")
}

fn build(env_file: &str, rewrite: impl Fn(String) -> String) -> World {
    let yaml = rewrite(std::fs::read_to_string(example().join(env_file)).unwrap());
    let manifest: EnvironmentManifest = serde_yaml::from_str(&yaml).unwrap();
    World::from_manifest(manifest, &example()).expect("world")
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

/// The same run on the other drivers: the lockstep round driver, one
/// `run_until_ps` call, and the per-node scheduler with idle fast-forward on.
/// Each must leave `fingerprint` as `run_us` on the default driver does.
fn same_on_every_driver<T: PartialEq + std::fmt::Debug>(
    env_file: &str,
    us: u64,
    fingerprint: impl Fn(&World) -> T,
) {
    let mut reference = build(env_file, |s| s);
    run_us(&mut reference, us);
    let want = fingerprint(&reference);

    let mut lockstep = build(env_file, |s| s);
    lockstep.set_gpio_lockstep(true);
    run_us(&mut lockstep, us);
    assert_eq!(fingerprint(&lockstep), want, "lockstep rounds");

    for ff in [false, true] {
        let mut until = build(env_file, |s| s);
        for m in until.machines.values_mut() {
            m.set_idle_fast_forward(ff);
        }
        for (id, r) in until.run_until_ps(us * 1_000_000).unwrap() {
            r.unwrap_or_else(|e| panic!("node {id}: {e:?}"));
        }
        assert_eq!(
            fingerprint(&until),
            want,
            "run_until_ps, idle fast-forward {ff}"
        );
    }

    let mut ff = build(env_file, |s| s);
    for m in ff.machines.values_mut() {
        m.set_idle_fast_forward(true);
    }
    run_us(&mut ff, us);
    assert_eq!(fingerprint(&ff), want, "rounds, idle fast-forward");
}

fn result(world: &World, id: &str, words: usize) -> Vec<u32> {
    let b = world.machines[id]
        .read_memory(0x2000_0100, words * 4)
        .unwrap();
    b.chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn net<'a>(reports: &'a [GpioNetReport], name: &str) -> &'a GpioNetReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no net {name}"))
}

fn no_contention(reports: &[GpioNetReport]) {
    for r in reports {
        assert_eq!(r.contention_events, 0, "{r:#?}");
        assert!(
            r.diagnostics
                .iter()
                .all(|d| d.code != GPIO_NET_CONTENTION && d.code != GPIO_NET_FLOATING),
            "{r:#?}"
        );
    }
}

/// Edges the MOSI wire carries for the master's bytes, from an idle-low
/// line: one per change of the bit level, MSB first.
fn data_edges(bytes: &[u8], idle: bool) -> u64 {
    edges_of(&bits_of(bytes, false), idle)
}

/// The bits of `bytes` in wire order.
fn bits_of(bytes: &[u8], lsb_first: bool) -> Vec<bool> {
    let mut out = Vec::new();
    for &b in bytes {
        for i in 0..8 {
            let pos = if lsb_first { i } else { 7 - i };
            out.push((b >> pos) & 1 != 0);
        }
    }
    out
}

/// Level changes along `levels`, starting from `idle`.
fn edges_of(levels: &[bool], idle: bool) -> u64 {
    let mut level = idle;
    let mut edges = 0;
    for &l in levels {
        if l != level {
            edges += 1;
            level = l;
        }
    }
    edges
}

/// What each node's pads saw, per node in delivery order, plus the net
/// totals: a fingerprint that must not depend on node order or on the
/// world's round size.
/// Per net: (name, edges, final level).
type NetTotals = Vec<(String, u64, bool)>;
/// Per delivery: (node, cycle, pin, level).
type Deliveries = Vec<(String, u64, u8, bool)>;

fn applied_fingerprint(world: &World) -> (NetTotals, Deliveries) {
    let nets = world
        .gpio_net_reports()
        .iter()
        .map(|n| (n.name.clone(), n.edges, n.level))
        .collect();
    // How deliveries to the nodes interleave in the log depends on the
    // round size; what each node sees does not.
    let mut applied: Vec<_> = world
        .gpio_net_applied()
        .iter()
        .map(|d| (d.node.clone(), d.cycle, d.pin, d.level))
        .collect();
    applied.sort();
    (nets, applied)
}

/// Run `env` three ways (as written, with its two nodes swapped, and with
/// 37 ns rounds) and require the same results and the same wire history.
fn assert_order_and_round_independent(
    env: &str,
    rewrite: impl Fn(String) -> String,
    nodes: [&str; 2],
    us: u64,
    words: usize,
) {
    let fingerprint = |world: &World| {
        (
            nodes.map(|n| result(world, n, words)),
            applied_fingerprint(world),
        )
    };
    let mut reference = build(env, &rewrite);
    run_us(&mut reference, us);
    let want = fingerprint(&reference);

    let mut reordered = build(env, |s| swap_nodes(&rewrite(s)));
    run_us(&mut reordered, us);
    assert_eq!(fingerprint(&reordered), want, "{env}: node order");

    let mut short = build(env, &rewrite);
    short.set_gpio_round_ps(37_000).unwrap();
    run_us(&mut short, us);
    assert_eq!(fingerprint(&short), want, "{env}: 37 ns rounds");
}

/// The longest time `pin` of `node` was held low, in that node's cycles, as
/// the net delivered it there: a stretched clock shows up here.
fn longest_low(world: &World, node: &str, pin: u8) -> u64 {
    let mut edges: Vec<_> = world
        .gpio_net_applied()
        .iter()
        .filter(|d| d.node == node && d.pin == pin)
        .map(|d| (d.cycle, d.level))
        .collect();
    edges.sort();
    let mut fell = None;
    let mut longest = 0;
    for (cycle, level) in edges {
        match (level, fell) {
            (false, _) => fell = Some(cycle),
            (true, Some(at)) => {
                longest = longest.max(cycle - at);
                fell = None;
            }
            _ => {}
        }
    }
    longest
}

/// Swap the first two node entries of an environment file.
fn swap_nodes(yaml: &str) -> String {
    let start = yaml.find("  - id: ").expect("first node");
    let second = start + 1 + yaml[start + 1..].find("  - id: ").expect("second node");
    let end = yaml[second..]
        .find("interconnects:")
        .map_or(yaml.len(), |i| second + i);
    let (a, b) = (&yaml[start..second], &yaml[second..end]);
    format!("{}{b}{a}{}", &yaml[..start], &yaml[end..])
}

mod spi {
    use super::*;

    const MASTER_TX: [u8; 4] = [0xA5, 0x3C, 0x5A, 0xC3];
    const SLAVE_TX: [u8; 4] = [0x81, 0x82, 0x83, 0x84];

    #[test]
    fn a_master_and_a_slave_exchange_four_bytes_bit_by_bit() {
        let mut world = build("env-spi.yaml", |s| s);
        run_us(&mut world, 400);

        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "master finished: {a:x?}");
        assert_eq!(
            &a[..4],
            SLAVE_TX.map(u32::from).as_slice(),
            "the master's DR holds what the slave put on MISO"
        );
        assert_eq!(a[4] & (1 << 6), 0, "master: no overrun");

        let b = result(&world, "b", 8);
        assert_eq!(
            &b[..4],
            MASTER_TX.map(u32::from).as_slice(),
            "the slave's DR holds what the master put on MOSI"
        );
        assert_eq!(b[4], 4, "one RXNE interrupt per byte");
        assert_eq!(b[5], 0, "no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // 4 frames x 8 bits x 2 SCK edges.
        assert_eq!(net(&reports, "sck").edges, 64);
        // Chip select: down once, up once.
        assert_eq!(net(&reports, "cs").edges, 2);
        assert!(net(&reports, "cs").level, "deselected at the end");
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // MISO: the four answers, then two more edges. The slave's interrupt
        // queues the next answer (0x85) after every byte, so the trailing
        // SCK edge of the last frame shifts its MSB (1) out; when CS rises the
        // slave lets go and the pull-down takes the wire back to 0.
        assert_eq!(
            net(&reports, "miso").edges,
            data_edges(&SLAVE_TX, false) + 2
        );
    }

    #[test]
    fn the_exchange_does_not_depend_on_node_order_or_round_size() {
        let fingerprint = |world: &World| {
            (
                result(world, "a", 8),
                result(world, "b", 8),
                world
                    .gpio_net_reports()
                    .iter()
                    .map(|n| (n.name.clone(), n.edges, n.level))
                    .collect::<Vec<_>>(),
                {
                    // Per node, in delivery order: how deliveries to the two
                    // nodes interleave in the log depends on the round size,
                    // what each node sees does not.
                    let mut applied: Vec<_> = world
                        .gpio_net_applied()
                        .iter()
                        .map(|d| (d.node.clone(), d.cycle, d.pin, d.level))
                        .collect();
                    applied.sort();
                    applied
                },
            )
        };
        let mut reference = build("env-spi.yaml", |s| s);
        run_us(&mut reference, 400);
        let want = fingerprint(&reference);

        let mut reordered = build("env-spi.yaml", |s| {
            let a = "  - id: a\n    system: \"system.yaml\"\n    firmware: \"firmware/spi-master.elf\"\n";
            let b = "  - id: b\n    system: \"system.yaml\"\n    firmware: \"firmware/spi-slave.elf\"\n";
            s.replace(&format!("{a}{b}"), &format!("{b}{a}"))
        });
        run_us(&mut reordered, 400);
        assert_eq!(fingerprint(&reordered), want, "node order");

        let mut short = build("env-spi.yaml", |s| s);
        short.set_gpio_round_ps(37_000).unwrap();
        run_us(&mut short, 400);
        assert_eq!(fingerprint(&short), want, "37 ns rounds");

        // Each node runs until its own pad drive changes, or its peer's
        // horizon: the same exchange on every driver.
        same_on_every_driver("env-spi.yaml", 400, |w| {
            (
                fingerprint(w),
                ["a", "b"].map(|id| w.machines[id].total_cycles()),
            )
        });
    }

    #[test]
    fn mosi_and_miso_crossed_at_the_slave_fight_and_the_net_says_so() {
        // A wiring mistake: the slave's MISO pad is on the master's MOSI wire
        // and its MOSI pad on the master's MISO wire. Two push-pull outputs
        // now share one wire while the slave is selected.
        let mut world = build("env-spi.yaml", |s| {
            s.replace(
                "{ node: b, peripheral: gpioa, pin: 6 }",
                "{ node: b, peripheral: gpioa, pin: TMP }",
            )
            .replace(
                "{ node: b, peripheral: gpioa, pin: 7 }",
                "{ node: b, peripheral: gpioa, pin: 6 }",
            )
            .replace(
                "{ node: b, peripheral: gpioa, pin: TMP }",
                "{ node: b, peripheral: gpioa, pin: 7 }",
            )
        });
        run_us(&mut world, 400);
        let reports = world.gpio_net_reports();
        let mosi = net(&reports, "mosi");
        assert!(mosi.contention_events > 0, "{mosi:#?}");
        let d = mosi
            .diagnostics
            .iter()
            .find(|d| d.code == GPIO_NET_CONTENTION)
            .expect("GPIO_NET_CONTENTION");
        let who: Vec<_> = d.members.iter().map(|m| (m.node.as_str(), m.pin)).collect();
        assert_eq!(who, vec![("a", 7), ("b", 6)]);
        // Nobody drives the master's MISO wire any more: it reads the pull.
        let a = result(&world, "a", 8);
        assert_eq!(&a[..4], &[0, 0, 0, 0]);
    }
}

/// SPI frame shapes (modes 1-3, LSB first) and the slave's transmit side
/// (TXE interrupt, underrun), each a pair of firmware variants built from
/// `src/spi_master.c` / `src/spi_slave.c` (`build.sh`).
mod spi_shapes {
    use super::*;

    const MASTER_TX: [u8; 4] = [0xA5, 0x3C, 0x5A, 0xC3];
    const SLAVE_TX: [u8; 4] = [0x81, 0x82, 0x83, 0x84];

    fn firmware(master: &'static str, slave: &'static str) -> impl Fn(String) -> String {
        move |s: String| {
            s.replace(
                "\"firmware/spi-master.elf\"",
                &format!("\"firmware/{master}.elf\""),
            )
            .replace(
                "\"firmware/spi-slave.elf\"",
                &format!("\"firmware/{slave}.elf\""),
            )
        }
    }

    /// One select, four frames. `answers` is what the slave shifts out;
    /// `after` the first bit of the word it holds next (a CPHA=0 slave puts
    /// it on MISO at the last frame's trailing edge).
    #[allow(clippy::too_many_arguments)]
    fn exchange(
        master: &'static str,
        slave: &'static str,
        cpol: bool,
        cpha: bool,
        lsb: bool,
        answers: [u8; 4],
        after: bool,
    ) -> World {
        let mut world = build("env-spi.yaml", firmware(master, slave));
        run_us(&mut world, 400);

        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "{master}: master finished: {a:x?}");
        assert_eq!(
            &a[..4],
            answers.map(u32::from).as_slice(),
            "{master}/{slave}: the master's DR holds what the slave put on MISO"
        );
        assert_eq!(a[4] & (1 << 6), 0, "master: no overrun");
        let b = result(&world, "b", 8);
        assert_eq!(
            &b[..4],
            MASTER_TX.map(u32::from).as_slice(),
            "{master}/{slave}: the slave's DR holds what the master put on MOSI"
        );
        assert_eq!(b[4], 4, "one RXNE interrupt per frame");
        assert_eq!(b[5], 0, "no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // CPOL=1: SCK rises once when the master is enabled and rests high.
        assert_eq!(
            net(&reports, "sck").edges,
            64 + u64::from(cpol),
            "{master}: 4 frames x 8 bits x 2 edges"
        );
        assert_eq!(net(&reports, "sck").level, cpol, "SCK rests at CPOL");
        assert_eq!(net(&reports, "cs").edges, 2);
        assert_eq!(
            net(&reports, "mosi").edges,
            edges_of(&bits_of(&MASTER_TX, lsb), false),
            "{master}: MOSI carries the master's bits in wire order"
        );
        // MISO: the answers' bits; at CPHA=0 the next word's first bit at the
        // last trailing edge; the pull-down once CS rises and the slave lets
        // go.
        let mut miso = bits_of(&answers, lsb);
        if !cpha {
            miso.push(after);
        }
        miso.push(false);
        assert_eq!(
            net(&reports, "miso").edges,
            edges_of(&miso, false),
            "{slave}: MISO"
        );
        assert!(!net(&reports, "miso").level);
        world
    }

    #[test]
    fn mode_1_shifts_on_the_leading_edge_and_samples_on_the_trailing_one() {
        exchange(
            "spi-master-mode1",
            "spi-slave-mode1",
            false,
            true,
            false,
            SLAVE_TX,
            true,
        );
    }

    #[test]
    fn mode_2_idles_the_clock_high() {
        exchange(
            "spi-master-mode2",
            "spi-slave-mode2",
            true,
            false,
            false,
            SLAVE_TX,
            true,
        );
    }

    #[test]
    fn mode_3_idles_high_and_samples_on_the_rising_edge() {
        exchange(
            "spi-master-mode3",
            "spi-slave-mode3",
            true,
            true,
            false,
            SLAVE_TX,
            true,
        );
    }

    #[test]
    fn lsb_first_frames_cross_bit_reversed_on_the_wire() {
        // 0x85, the next answer, is 1 in its LSB too.
        exchange(
            "spi-master-lsb",
            "spi-slave-lsb",
            false,
            false,
            true,
            SLAVE_TX,
            true,
        );
    }

    #[test]
    fn a_slave_fed_from_its_txe_interrupt_answers_every_frame() {
        let world = exchange(
            "spi-master",
            "spi-slave-txe",
            false,
            false,
            false,
            SLAVE_TX,
            // Nothing queued after 0x84: the shift register holds the last
            // frame received (0xC3), whose MSB goes out.
            true,
        );
        let b = result(&world, "b", 8);
        assert_eq!(b[6], 4, "TXE interrupts: one per answer, then TXEIE off");
    }

    #[test]
    fn an_underrun_slave_shifts_out_the_frame_it_received_last() {
        // The slave writes 0x81 once and never again: each later frame
        // starts with an empty transmit buffer, and the shift register still
        // holds the previous frame from MOSI.
        exchange(
            "spi-master",
            "spi-slave-underrun",
            false,
            false,
            false,
            [0x81, 0xA5, 0x3C, 0x5A],
            true,
        );
    }

    #[test]
    fn the_frame_shapes_do_not_depend_on_node_order_or_round_size() {
        for (master, slave) in [
            ("spi-master-mode3", "spi-slave-mode3"),
            ("spi-master-lsb", "spi-slave-lsb"),
            ("spi-master", "spi-slave-txe"),
        ] {
            assert_order_and_round_independent(
                "env-spi.yaml",
                firmware(master, slave),
                ["a", "b"],
                400,
                8,
            );
        }
    }
}

/// 16-bit frames (CR1.DFF) between two STM32F401s (`env-spi16.yaml`).
mod spi16 {
    use super::*;

    const MASTER_TX: [u16; 3] = [0xA55A, 0x0FF0, 0x8001];
    const SLAVE_TX: [u16; 3] = [0x1234, 0xBEEF, 0x00FF];

    fn bits16(words: &[u16]) -> Vec<bool> {
        words
            .iter()
            .flat_map(|w| (0..16).rev().map(move |i| (w >> i) & 1 != 0))
            .collect()
    }

    #[test]
    fn sixteen_bit_frames_cross_whole() {
        let mut world = build("env-spi16.yaml", |s| s);
        run_us(&mut world, 400);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "master finished: {a:x?}");
        assert_eq!(&a[..3], SLAVE_TX.map(u32::from).as_slice());
        assert_eq!(a[4] & (1 << 6), 0, "master: no overrun");
        let b = result(&world, "b", 8);
        assert_eq!(&b[..3], MASTER_TX.map(u32::from).as_slice());
        assert_eq!(b[4], 3, "one RXNE interrupt per 16-bit frame");
        assert_eq!(b[5], 0, "no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 3 * 16 * 2);
        assert_eq!(
            net(&reports, "mosi").edges,
            edges_of(&bits16(&MASTER_TX), false)
        );
        // Selected in software from enable: the slave drives MISO from then
        // on; its next word (0x7E7E) shows its MSB (0) at the last trailing
        // edge.
        let mut miso = bits16(&SLAVE_TX);
        miso.push(false);
        assert_eq!(net(&reports, "miso").edges, edges_of(&miso, false));
    }

    #[test]
    fn sixteen_bit_frames_do_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-spi16.yaml", |s| s, ["a", "b"], 400, 8);
    }
}

mod i2c {
    use super::*;

    #[test]
    fn a_controller_writes_and_reads_back_a_target_over_open_drain_nets() {
        let mut world = build("env-i2c.yaml", |s| s);
        run_us(&mut world, 1500);

        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?}");
        assert_eq!(a[2], 0, "0x42 ACKed the write");
        assert_eq!(&a[..2], &[0xDE, 0xAD], "read back through a repeated START");
        assert_eq!(a[3], 1, "nobody at 0x50: NACKF");
        assert_eq!(a[4], 3, "three transfers, three STOPs");

        let b = result(&world, "b", 8);
        assert_eq!(b[0], 2, "the target saw its two transfers end");
        assert_eq!(&b[1..3], &[0xDE, 0xAD], "stored what the controller wrote");
        assert_eq!(b[3], 4, "pointer + 2 data, then the pointer again");
        assert!(b[4] >= 2, "sent at least the two bytes read: {b:x?}");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let scl = net(&reports, "scl");
        assert!(
            scl.level && net(&reports, "sda").level,
            "bus idle at the end"
        );
        // Ten bytes of 9 clocks (address + 3; address + 1, address + 2;
        // address), one repeated-START pulse and three STOP pulses. Every
        // clock is one fall and one rise; stretching delays, never adds.
        let clocks = 9 * 10 + 1 + 3;
        assert_eq!(scl.edges, 2 * clocks, "{scl:#?}");
    }

    #[test]
    fn a_slow_target_holds_the_controller_by_stretching_scl() {
        // The target's handler takes several SCL low periods to answer ADDR
        // and TXIS: the controller must wait on the stretched clock, and
        // the bytes and the clock count stay what they were.
        let mut world = build("env-i2c.yaml", |s| {
            s.replace("firmware/i2c-target.elf", "firmware/i2c-target-slow.elf")
        });
        run_us(&mut world, 3000);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?}");
        assert_eq!((a[2], a[3], a[4]), (0, 1, 3), "{a:x?}");
        assert_eq!(&a[..2], &[0xDE, 0xAD], "read back through the stretch");
        let b = result(&world, "b", 8);
        assert_eq!(&b[1..4], &[0xDE, 0xAD, 4], "{b:x?}");
        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "scl").edges, 2 * (9 * 10 + 1 + 3));
        // The controller's own SCL low period is (SCLL + 1) x 8 = 128 cycles.
        let held = longest_low(&world, "a", 6);
        assert!(held > 4 * 128, "SCL held low {held} cycles");
    }

    #[test]
    fn with_no_target_on_the_bus_every_address_is_nacked() {
        // Node b runs the SPI slave firmware: its I2C1 is never enabled, so
        // nothing ACKs 0x42 or 0x50. The pull-up holds SDA high through every
        // ACK slot and the controller sees NACKF and sends STOP each time.
        let mut world = build("env-i2c.yaml", |s| {
            s.replace("firmware/i2c-target.elf", "firmware/spi-slave.elf")
        });
        run_us(&mut world, 1500);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "{a:x?}");
        assert_eq!(a[2], 1, "write to 0x42 NACKed");
        assert_eq!(&a[..2], &[0xEE, 0xEE], "the read was skipped");
        assert_eq!(a[3], 1, "write to 0x50 NACKed");
        assert_eq!(a[4], 3, "every NACK ends in a STOP");
        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // Three address frames (9 clocks) and three STOP pulses.
        assert_eq!(net(&reports, "scl").edges, 2 * (3 * 9 + 3));
    }

    #[test]
    fn the_transfers_do_not_depend_on_round_size() {
        let fingerprint = |world: &World| {
            let mut applied: Vec<_> = world
                .gpio_net_applied()
                .iter()
                .map(|d| (d.node.clone(), d.cycle, d.pin, d.level))
                .collect();
            applied.sort();
            (result(world, "a", 8), result(world, "b", 8), applied)
        };
        let mut reference = build("env-i2c.yaml", |s| s);
        run_us(&mut reference, 1500);
        let mut short = build("env-i2c.yaml", |s| s);
        short.set_gpio_round_ps(23_000).unwrap();
        run_us(&mut short, 1500);
        assert_eq!(fingerprint(&short), fingerprint(&reference));

        same_on_every_driver("env-i2c.yaml", 1500, |w| {
            (
                fingerprint(w),
                ["a", "b"].map(|id| w.machines[id].total_cycles()),
            )
        });
    }
}

/// The modern controller beyond 7-bit/255 bytes (`i2c_bulk_*` firmware on
/// the `env-i2c.yaml` wiring): 10-bit addressing both ways, RELOAD, the
/// general call and a masked OAR2.
mod i2c_bulk {
    use super::*;

    fn bulk(s: String) -> String {
        s.replace(
            "firmware/i2c-controller.elf",
            "firmware/i2c-bulk-controller.elf",
        )
        .replace("firmware/i2c-target.elf", "firmware/i2c-bulk-target.elf")
    }

    #[test]
    fn three_hundred_bytes_cross_each_way_over_a_ten_bit_address() {
        let mut world = build("env-i2c.yaml", bulk);
        run_us(&mut world, 12_000);

        let sent: u32 = (0..300u32).map(|i| (3 * i + 7) & 0xFF).sum();
        let read: u32 = (0..300u32).map(|i| (5 * i + 1) & 0xFF).sum();
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?}");
        assert_eq!(a[0], 0, "0x2A5 ACKed 300 bytes over two RELOAD chunks");
        assert_eq!((a[2], a[1]), (300, read), "300 bytes read back");
        assert_eq!(a[3], 1, "the 0x1A5 header is nobody's: NACKF");
        assert_eq!(a[5], 0, "the general call was ACKed");
        assert_eq!(a[6], 0, "0x31 matched OAR2 0x30 with one bit masked");
        assert_eq!(a[4], 5, "five transfers, five STOPs");

        let b = result(&world, "b", 16);
        assert_eq!(
            (b[0], b[1], b[2]),
            (300, sent, 0),
            "received in order: {b:x?}"
        );
        assert!(b[3] >= 300, "loaded at least the 300 bytes read: {b:x?}");
        assert_eq!(b[4], 4, "STOPF after each transfer it took part in");
        assert_eq!((b[5], b[6]), (0x06, 0x5C));
        // ADDCODE (| DIR << 7) per ADDR: 10-bit header code 11110 10 for the
        // write, again for the read's write half and with DIR for its read
        // half, then the general call and the OAR2 address.
        assert_eq!(&b[8..13], &[0x7A, 0x7A, 0xFA, 0x00, 0x31]);

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let scl = net(&reports, "scl");
        assert!(
            scl.level && net(&reports, "sda").level,
            "bus idle at the end"
        );
        // 610 bytes of 9 clocks (header + low + 300; header + low + header +
        // 300; header; address + 1, twice), one repeated START, five STOPs.
        // RELOAD stretches, never adds a clock.
        assert_eq!(scl.edges, 2 * (610 * 9 + 1 + 5), "{scl:#?}");
    }

    #[test]
    fn the_bulk_transfers_do_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-i2c.yaml", bulk, ["a", "b"], 12_000, 16);
    }
}

/// The legacy STM32 I²C (F1/F2/F4 CR1/SR1/SR2/CCR register file) over nets:
/// an STM32F103 and an STM32F401 (`env-i2c-legacy.yaml`), both ways round,
/// and against the modern controller of an STM32G071.
mod legacy_i2c {
    use super::*;

    /// The F401 is the controller, the F103 the target.
    fn swapped(s: String) -> String {
        s.replace("system-f103.yaml", "SYS_A")
            .replace("system-f401.yaml", "system-f103.yaml")
            .replace("SYS_A", "system-f401.yaml")
            .replace("f1-legacy-i2c-controller", "f4-legacy-i2c-controller")
            .replace("f4-legacy-i2c-target", "f1-legacy-i2c-target")
    }

    fn check_register_file_exchange(world: &World) {
        let a = result(world, "a", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?}");
        assert_eq!(a[2], 0, "0x42 ACKed the write");
        assert_eq!(&a[..2], &[0xDE, 0xAD], "read back after a repeated START");
        // The byte the target loaded when the 2-byte read was NACKed
        // (regs[0x12] = 0x37) stayed in its DR and leads the next read.
        assert_eq!(a[5], 0x37, "single-byte read: {a:x?}");
        assert_eq!(a[3], 1, "nobody at 0x50: AF");
        assert_eq!(a[4], 4, "four transfers, four STOPs");

        let b = result(world, "b", 8);
        assert_eq!(b[0], 1, "STOPF after the one write ended by STOP: {b:x?}");
        assert_eq!(&b[1..3], &[0xDE, 0xAD], "stored what the controller wrote");
        assert_eq!(b[3], 4, "pointer + 2 data, then the pointer again");
        assert_eq!(b[4], 4, "DR loads: 0x10..0x13");
        assert_eq!(b[5], 2, "both reads ended by the controller's NACK (AF)");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let scl = net(&reports, "scl");
        assert!(
            scl.level && net(&reports, "sda").level,
            "bus idle at the end"
        );
        // Twelve bytes of 9 clocks (address + 3; address + 1, address + 2;
        // address + 1; address), one repeated-START pulse, four STOP pulses.
        assert_eq!(scl.edges, 2 * (9 * 12 + 1 + 4), "{scl:#?}");
    }

    #[test]
    fn an_f103_controller_writes_and_reads_an_f401_target() {
        let mut world = build("env-i2c-legacy.yaml", |s| s);
        run_us(&mut world, 3000);
        check_register_file_exchange(&world);
    }

    #[test]
    fn an_f401_controller_writes_and_reads_an_f103_target() {
        let mut world = build("env-i2c-legacy.yaml", swapped);
        run_us(&mut world, 3000);
        check_register_file_exchange(&world);
    }

    #[test]
    fn a_slow_slave_holds_the_master_by_stretching_scl() {
        // The F401's handler takes about 10 us to clear ADDR, four SCL low
        // periods of the F103: the F103 must wait on the stretched clock.
        let mut world = build("env-i2c-legacy.yaml", |s| {
            s.replace("f4-legacy-i2c-target.elf", "f4-legacy-i2c-target-slow.elf")
        });
        run_us(&mut world, 3000);
        check_register_file_exchange(&world);
        // The F103's own SCL low period is CCR = 180 cycles.
        let held = longest_low(&world, "a", 6);
        assert!(held > 3 * 180, "SCL held low {held} cycles");
    }

    #[test]
    fn with_no_target_every_address_is_nacked() {
        // Node b's I2C1 is never enabled: the pull-up reads 1 in every ACK
        // slot.
        let mut world = build("env-i2c-legacy.yaml", |s| {
            s.replace("f4-legacy-i2c-target", "f4-spi16-slave")
        });
        run_us(&mut world, 3000);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "{a:x?}");
        assert_eq!(a[2], 1, "write to 0x42 NACKed");
        assert_eq!(&a[..2], &[0xEE, 0xEE], "the read was skipped");
        assert_eq!(a[5], 0xEE, "the single-byte read was NACKed");
        assert_eq!(a[3], 1, "write to 0x50 NACKed");
        assert_eq!(a[4], 4, "every AF ends in a STOP");
        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // Four address frames (9 clocks) and four STOP pulses.
        assert_eq!(net(&reports, "scl").edges, 2 * (4 * 9 + 4));
    }

    #[test]
    fn a_modern_g071_controller_talks_to_a_legacy_f103_target() {
        // The G071 controller firmware of `env-i2c.yaml` against the F103
        // register-file target: the two register generations meet only on
        // the wire.
        let mut world = build("env-i2c.yaml", |s| {
            s.replace(
                "    system: \"system.yaml\"\n    firmware: \"firmware/i2c-target.elf\"",
                "    system: \"system-f103.yaml\"\n    firmware: \"firmware/f1-legacy-i2c-target.elf\"",
            )
        });
        run_us(&mut world, 1500);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?}");
        assert_eq!(a[2], 0, "0x42 ACKed the write");
        assert_eq!(&a[..2], &[0xDE, 0xAD]);
        assert_eq!(a[3], 1, "nobody at 0x50: NACKF");
        assert_eq!(a[4], 3);
        let b = result(&world, "b", 8);
        assert_eq!(b[0], 1, "STOPF for the write ended by STOP: {b:x?}");
        assert_eq!(&b[1..3], &[0xDE, 0xAD]);
        assert_eq!(b[3], 4);
        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "scl").edges, 2 * (9 * 10 + 1 + 3));
    }

    #[test]
    fn the_transfers_do_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-i2c-legacy.yaml", |s| s, ["a", "b"], 3000, 8);
        assert_order_and_round_independent("env-i2c-legacy.yaml", swapped, ["a", "b"], 3000, 8);
    }
}

/// The RP2040's PL022 SSP against the STM32G071's SPI, both ways round
/// (`env-spi-rp2040.yaml`, `env-spi-rp2040-slave.yaml`).
mod rp2040_spi {
    use super::*;

    const MASTER_TX: [u8; 4] = [0xA5, 0x3C, 0x5A, 0xC3];
    const SLAVE_TX: [u8; 4] = [0x81, 0x82, 0x83, 0x84];

    #[test]
    fn an_rp2040_master_frames_each_byte_with_its_chip_select() {
        let mut world = build("env-spi-rp2040.yaml", |s| s);
        run_us(&mut world, 400);
        let rp = result(&world, "rp", 8);
        assert_eq!(rp[7], 1, "master finished: {rp:x?}");
        assert_eq!(&rp[..4], SLAVE_TX.map(u32::from).as_slice());
        assert_eq!(rp[4], 0x3, "SSPSR: TX FIFO empty, not busy, RX drained");
        let b = result(&world, "b", 8);
        assert_eq!(&b[..4], MASTER_TX.map(u32::from).as_slice());
        assert_eq!((b[4], b[5]), (4, 0), "one RXNE per frame, no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 64);
        // SPH = 0: CSn goes high between frames, so four frames are four
        // low pulses.
        assert_eq!(net(&reports, "cs").edges, 8);
        assert!(net(&reports, "cs").level);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // MISO: each answer; at its last trailing edge the slave's next
        // word's MSB (1 for 0x82..0x85); released to the pull-down while CS
        // is high; the next frame starts from that MSB again.
        let mut miso = Vec::new();
        for byte in SLAVE_TX {
            miso.extend(bits_of(&[byte], false));
            miso.push(true);
            miso.push(false);
        }
        assert_eq!(net(&reports, "miso").edges, edges_of(&miso, false));
    }

    #[test]
    fn an_rp2040_slave_answers_from_its_tx_fifo() {
        let mut world = build("env-spi-rp2040-slave.yaml", |s| s);
        run_us(&mut world, 400);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "master finished: {a:x?}");
        assert_eq!(&a[..4], SLAVE_TX.map(u32::from).as_slice());
        let rp = result(&world, "rp", 8);
        assert_eq!(rp[7], 1, "slave finished: {rp:x?}");
        assert_eq!(&rp[..5], &[0xA5, 0x3C, 0x5A, 0xC3, 4]);

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(
            net(&reports, "sck").edges,
            64 + 1,
            "mode 3: SCK rises at enable"
        );
        assert_eq!(net(&reports, "cs").edges, 2);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        let mut miso = bits_of(&SLAVE_TX, false);
        miso.push(false);
        assert_eq!(net(&reports, "miso").edges, edges_of(&miso, false));
    }

    #[test]
    fn rp2040_exchanges_do_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-spi-rp2040.yaml", |s| s, ["rp", "b"], 400, 8);
        assert_order_and_round_independent("env-spi-rp2040-slave.yaml", |s| s, ["a", "rp"], 400, 8);
    }
}

/// The RP2040's DW_apb_i2c against the STM32G071's modern I²C, both ways
/// round (`env-i2c-rp2040.yaml`, `env-i2c-rp2040-target.yaml`): the same
/// register-file exchange as `env-i2c.yaml`.
mod rp2040_i2c {
    use super::*;

    /// Ten bytes of 9 clocks (address + 3; address + 1, address + 2;
    /// address), one repeated START, three STOPs.
    const SCL_EDGES: u64 = 2 * (9 * 10 + 1 + 3);

    #[test]
    fn an_rp2040_controller_writes_and_reads_a_g071_target() {
        let mut world = build("env-i2c-rp2040.yaml", |s| s);
        run_us(&mut world, 1500);
        let rp = result(&world, "rp", 8);
        assert_eq!(rp[7], 1, "controller finished: {rp:x?}");
        assert_eq!(rp[2], 0, "0x42 ACKed the write");
        assert_eq!(
            &rp[..2],
            &[0xDE, 0xAD],
            "read back after a RESTART: {rp:x?} {:x?}",
            result(&world, "b", 8)
        );
        assert_eq!(rp[3], 1, "nobody at 0x50: TX_ABRT");
        assert_eq!(rp[5] & 1, 1, "ABRT_7B_ADDR_NOACK");
        assert_eq!(rp[4], 3, "three STOP_DETs");

        let b = result(&world, "b", 8);
        assert_eq!(b[0], 2, "the target saw its two transfers end");
        assert_eq!(&b[1..3], &[0xDE, 0xAD]);
        assert_eq!(b[3], 4, "pointer + 2 data, then the pointer again");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let scl = net(&reports, "scl");
        assert!(
            scl.level && net(&reports, "sda").level,
            "bus idle at the end"
        );
        assert_eq!(scl.edges, SCL_EDGES, "{scl:#?}");
    }

    #[test]
    fn a_g071_controller_writes_and_reads_an_rp2040_target() {
        let mut world = build("env-i2c-rp2040-target.yaml", |s| s);
        run_us(&mut world, 1500);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?}");
        assert_eq!(a[2], 0, "0x42 ACKed the write");
        assert_eq!(&a[..2], &[0xDE, 0xAD], "read back through a repeated START");
        assert_eq!(a[3], 1, "nobody at 0x50: NACKF");
        assert_eq!(a[4], 3);

        let rp = result(&world, "rp", 8);
        assert_eq!(rp[0], 2, "STOP_DET for its two transfers only: {rp:x?}");
        assert_eq!(&rp[1..3], &[0xDE, 0xAD]);
        assert_eq!(rp[3], 4, "pointer + 2 data, then the pointer again");
        assert_eq!(rp[4], 2, "one RD_REQ per byte read");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let scl = net(&reports, "scl");
        assert!(scl.level && net(&reports, "sda").level);
        assert_eq!(scl.edges, SCL_EDGES, "{scl:#?}");
    }

    #[test]
    fn a_slow_rp2040_target_holds_the_controller_by_stretching_scl() {
        // About 17 us before each read answer, eight G071 SCL low periods:
        // the RP2040 holds SCL low with RD_REQ until it writes the byte.
        let mut world = build("env-i2c-rp2040-target.yaml", |s| {
            s.replace("rp2040-i2c-target.elf", "rp2040-i2c-target-slow.elf")
        });
        run_us(&mut world, 1500);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?}");
        assert_eq!(&a[..2], &[0xDE, 0xAD], "read back through the stretch");
        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(
            net(&reports, "scl").edges,
            SCL_EDGES,
            "stretching adds no clock"
        );
        // The G071's own SCL low period is (SCLL + 1) x 8 = 128 cycles.
        let held = longest_low(&world, "a", 6);
        assert!(held > 4 * 128, "SCL held low {held} cycles");
    }

    #[test]
    fn rp2040_i2c_does_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-i2c-rp2040.yaml", |s| s, ["rp", "b"], 1500, 8);
        assert_order_and_round_independent(
            "env-i2c-rp2040-target.yaml",
            |s| s,
            ["a", "rp"],
            1500,
            8,
        );
    }
}
