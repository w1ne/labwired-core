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

/// Where a node's firmware leaves its results: SRAM 0x2000_0100 on the
/// STM32s and the RP2040; on the ESP nodes (named `esp...`) DRAM
/// 0x3FC8_0100 on an ESP32-C3, HP SRAM 0x4080_0100 on an ESP32-C6, DRAM
/// 0x3FCE_0100 on an ESP32-S3 and SRAM2 0x3FFC_0100 on a classic ESP32 —
/// the first of them the node's memory map has.
fn result_base(world: &World, id: &str) -> u32 {
    if !id.starts_with("esp") {
        return 0x2000_0100;
    }
    let m = &world.machines[id];
    [0x3FC8_0100, 0x4080_0100, 0x3FCE_0100, 0x3FFC_0100]
        .into_iter()
        .find(|&base| m.read_memory(base, 4).is_ok())
        .expect("an ESP result area")
}

fn result(world: &World, id: &str, words: usize) -> Vec<u32> {
    let b = world.machines[id]
        .read_memory(result_base(world, id), words * 4)
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

/// SPI with DMA on both ends (`src/spi_dma_master.c`, `src/spi_dma_slave.c`):
/// SPI1's TXDMAEN / RXDMAEN requests reach DMA1 through the G0's DMAMUX, and
/// neither CPU touches DR. The bytes still cross bit by bit at the pads.
mod spi_dma {
    use super::*;

    pub(super) const MASTER_TX: [u8; 8] = [0xA5, 0x3C, 0x5A, 0xC3, 0x0F, 0xF0, 0x81, 0x7E];
    pub(super) const SLAVE_TX: [u8; 8] = [0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88];
    /// DMA1_ISR with channels 1 and 2 complete: GIF | TCIF | HTIF on each.
    pub(super) const ISR_BOTH_DONE: u32 = 0x77;

    #[test]
    fn a_dma_master_and_a_dma_slave_exchange_a_buffer_bit_by_bit() {
        let mut world = build("env-spi-dma.yaml", |s| s);
        run_us(&mut world, 1500);

        let a = result(&world, "a", 16);
        assert_eq!(a[15], 1, "master finished: {a:x?}");
        assert_eq!(
            &a[..8],
            SLAVE_TX.map(u32::from).as_slice(),
            "the master's RX channel moved what the slave put on MISO"
        );
        assert_eq!(a[8], ISR_BOTH_DONE, "both channels complete");
        assert_eq!((a[9], a[10]), (0, 0), "CNDTR drained");
        assert_eq!(
            a[11] & 0x1FFF,
            0x0002,
            "SR: TXE, FIFOs empty, no OVR, not busy"
        );

        let b = result(&world, "b", 16);
        assert_eq!(b[15], 1, "slave finished: {b:x?}");
        assert_eq!(
            &b[..8],
            MASTER_TX.map(u32::from).as_slice(),
            "the slave's RX channel moved what the master put on MOSI"
        );
        assert_eq!(b[8], ISR_BOTH_DONE);
        assert_eq!((b[9], b[10]), (0, 0));
        assert_eq!(b[11] & (1 << 6), 0, "no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // 8 frames x 8 bits x 2 SCK edges, back to back.
        assert_eq!(net(&reports, "sck").edges, 128);
        assert_eq!(net(&reports, "cs").edges, 2);
        assert!(net(&reports, "cs").level, "deselected at the end");
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // MISO: the eight answers. After the last one the slave's FIFO is
        // empty and it shifts its last received bit pattern (MSB of 0x7E,
        // a 0), then the pull-down: no further edge, as 0x88 ends in 0.
        assert_eq!(net(&reports, "miso").edges, data_edges(&SLAVE_TX, false));
    }

    #[test]
    fn the_dma_exchange_does_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-spi-dma.yaml", |s| s, ["a", "b"], 1500, 16);
    }

    /// STM32F401 DMA2 (src/f4_spi_dma_master.c): two memory-to-memory
    /// streams started together complete in priority order, not start order;
    /// then SPI1 moves both directions through the stream FIFOs in half-word
    /// INCR4 bursts against the G071 DMA slave, byte-exact over the nets.
    #[test]
    fn f4_streams_complete_by_priority_and_spi_moves_through_fifo_bursts() {
        let mut world = build("env-spi-f4-dma.yaml", |s| s);
        run_us(&mut world, 2500);

        let a = result(&world, "a", 16);
        assert_eq!(a[15], 1, "master finished: {a:x?}");
        assert_eq!(
            (a[0], a[1]),
            (1, 0),
            "stream 1 (PL very high) completes before stream 0 (PL low)"
        );
        // Stream 0 started first (NDTR < 64) and then stood still while
        // stream 1 ran: sharing the slots, it would have finished first.
        assert!(
            (1..64).contains(&a[2]),
            "stream 0 waited while stream 1 ran: NDTR {} of 64 left",
            a[2]
        );
        assert_eq!(a[3], 1, "both memory-to-memory copies byte-exact");
        assert_eq!(
            &a[4..12],
            SLAVE_TX.map(u32::from).as_slice(),
            "the RX stream packed what the slave put on MISO"
        );
        // LISR: TCIF and HTIF of streams 2 and 3, no FEIF / TEIF / DMEIF.
        assert_eq!(a[12], (0b11 << 20) | (0b11 << 26), "LISR {:#x}", a[12]);
        assert_eq!(a[13], 0, "both NDTR drained");
        assert_eq!(a[14] & 0x1C3, 0x2, "SR: TXE, not busy, no OVR / RXNE");

        let b = result(&world, "b", 16);
        assert_eq!(b[15], 1, "slave finished: {b:x?}");
        assert_eq!(
            &b[..8],
            MASTER_TX.map(u32::from).as_slice(),
            "the TX stream unpacked its half-word burst onto MOSI"
        );
        assert_eq!(b[11] & (1 << 6), 0, "no OVR on the slave");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 128);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
    }

    #[test]
    fn the_f4_fifo_exchange_does_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-spi-f4-dma.yaml", |s| s, ["a", "b"], 2500, 16);
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

/// The F401 pair of `spi16` on its hardware slave select
/// (`env-spi16-nss.yaml`): the master's SPI drives PA4 = SPI1_NSS
/// (CR2.SSOE=1) and the slave (SSM=0) is selected by the level its own PA4
/// = SPI1_NSS pad reads off the net.
mod spi16_nss {
    use super::*;

    const MASTER_TX: [u16; 3] = [0xA55A, 0x0FF0, 0x8001];
    const SLAVE_TX: [u16; 3] = [0x1234, 0xBEEF, 0x00FF];

    #[test]
    fn the_masters_ssoe_drives_nss_and_the_slave_follows_its_nss_pad() {
        let mut world = build("env-spi16-nss.yaml", |s| s);
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
        // NSS: driven low by SPE=1 with SSOE, released at SPE=0 (pull-up).
        assert_eq!(net(&reports, "nss").edges, 2);
        assert!(net(&reports, "nss").level, "released at the end");
        assert_eq!(net(&reports, "sck").edges, 3 * 16 * 2);
        // MOSI: the words, then the pull-down once SPE=0 stops the master
        // driving it (0x8001 left it high).
        let mut mosi = bits_n(&MASTER_TX, 16);
        mosi.push(false);
        assert_eq!(net(&reports, "mosi").edges, edges_of(&mosi, false));
        // Selected by the NSS fall: MISO carries the three answers, the MSB
        // (0) of the next word (0x7E7E) at the last trailing edge, then the
        // pull-down once NSS rises and the slave lets go.
        let mut miso = bits_n(&SLAVE_TX, 16);
        miso.push(false);
        miso.push(false);
        assert_eq!(net(&reports, "miso").edges, edges_of(&miso, false));
        assert!(!net(&reports, "miso").level);
    }

    #[test]
    fn with_nss_unwired_the_hardware_select_slave_never_answers() {
        // Without the NSS net the slave's pad reads no level: it is never
        // selected, takes no frame and leaves MISO to the pull-down.
        let mut world = build("env-spi16-nss.yaml", |s| {
            let start = s.find("  - type: gpio_net").unwrap();
            let end = start + 1 + s[start + 1..].find("  - type: gpio_net").unwrap();
            format!("{}{}", &s[..start], &s[end..])
        });
        run_us(&mut world, 400);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "master finished: {a:x?}");
        assert_eq!(&a[..3], &[0, 0, 0], "MISO is the pull-down");
        let b = result(&world, "b", 8);
        assert_eq!(b[4], 0, "the slave was never selected");
    }

    #[test]
    fn hardware_nss_frames_do_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-spi16-nss.yaml", |s| s, ["a", "b"], 400, 8);
    }
}

/// The bits of `words`, `n` bits each, MSB first.
fn bits_n(words: &[u16], n: u8) -> Vec<bool> {
    words
        .iter()
        .flat_map(|w| (0..n).rev().map(move |i| (w >> i) & 1 != 0))
        .collect()
}

/// The STM32G071's FIFO-type SPI (RM0444 §35) on the `env-spi.yaml` pair at
/// other CR2.DS frame sizes, each way round: `spi-*-ds16/12/4` (build.sh).
/// The 8-bit size is `spi` / `spi_shapes` above.
mod spi_frame_sizes {
    use super::*;

    fn mask(ds: u8) -> u16 {
        ((1u32 << ds) - 1) as u16
    }

    /// What `src/spi_master.c` sends at `ds` bits.
    fn master_tx(ds: u8) -> [u16; 4] {
        let words = if ds <= 8 {
            [0xA5, 0x3C, 0x5A, 0xC3]
        } else {
            [0xA55A, 0x3CC3, 0x5AA5, 0xC33C]
        };
        words.map(|w| w & mask(ds))
    }

    /// `ANSWER(n)` of `src/spi_slave.c`: 0x81+n in both bytes, masked.
    fn answer(ds: u8, n: u16) -> u16 {
        (((0x81 + n) * 0x0101) as u32 & u32::from(mask(ds))) as u16
    }

    /// Node `a` runs the master and `b` the slave, or the other way round
    /// (`swapped`); the wiring is symmetric, PA4 being the master's GPIO chip
    /// select and the slave's hardware NSS on either chip.
    fn env(ds: u8, swapped: bool) -> impl Fn(String) -> String {
        move |s: String| {
            let (master, slave) = (
                format!("\"firmware/spi-master-ds{ds}.elf\""),
                format!("\"firmware/spi-slave-ds{ds}.elf\""),
            );
            let (on_a, on_b) = if swapped {
                (slave, master)
            } else {
                (master, slave)
            };
            s.replace("\"firmware/spi-master.elf\"", "MASTER")
                .replace("\"firmware/spi-slave.elf\"", "SLAVE")
                .replace("MASTER", &on_a)
                .replace("SLAVE", &on_b)
        }
    }

    fn exchange(ds: u8, swapped: bool) {
        let mut world = build("env-spi.yaml", env(ds, swapped));
        run_us(&mut world, 600);
        let (m, sl) = if swapped { ("b", "a") } else { ("a", "b") };
        let answers = [0, 1, 2, 3].map(|n| answer(ds, n));
        let tx = master_tx(ds);

        let mr = result(&world, m, 8);
        assert_eq!(mr[7], 1, "DS={ds}: master finished: {mr:x?}");
        assert_eq!(
            &mr[..4],
            answers.map(u32::from).as_slice(),
            "DS={ds}: the master's RX FIFO holds what the slave put on MISO"
        );
        assert_eq!(mr[4] & (1 << 6), 0, "master: no overrun");
        // SR at the end: TXE only — both FIFOs empty (FRLVL = FTLVL = 0).
        assert_eq!(mr[4], 0x0002, "DS={ds}: master SR");
        let sr = result(&world, sl, 8);
        assert_eq!(
            &sr[..4],
            tx.map(u32::from).as_slice(),
            "DS={ds}: the slave's RX FIFO holds what the master put on MOSI"
        );
        assert_eq!(sr[4], 4, "DS={ds}: one RXNE interrupt per frame");
        assert_eq!(sr[5], 0, "no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let bits = u64::from(ds);
        assert_eq!(net(&reports, "sck").edges, 4 * bits * 2, "DS={ds}: SCK");
        assert_eq!(net(&reports, "cs").edges, 2);
        assert_eq!(
            net(&reports, "mosi").edges,
            edges_of(&bits_n(&tx, ds), false),
            "DS={ds}: MOSI"
        );
        // MISO: the answers, the MSB of the next one (queued from the RXNE
        // interrupt) at the last trailing edge, the pull-down once CS rises.
        let mut miso = bits_n(&answers, ds);
        miso.push((answer(ds, 4) >> (ds - 1)) & 1 != 0);
        miso.push(false);
        assert_eq!(
            net(&reports, "miso").edges,
            edges_of(&miso, false),
            "DS={ds}: MISO"
        );
    }

    #[test]
    fn sixteen_bit_frames_cross_whole_both_ways() {
        exchange(16, false);
        exchange(16, true);
    }

    #[test]
    fn twelve_bit_frames_cross_whole_both_ways() {
        exchange(12, false);
        exchange(12, true);
    }

    #[test]
    fn four_bit_frames_cross_whole_both_ways() {
        exchange(4, false);
        exchange(4, true);
    }

    #[test]
    fn eight_bit_frames_cross_both_ways() {
        // The default firmware is the DS=8 build; swap its roles too.
        let mut world = build("env-spi.yaml", |s| {
            s.replace("\"firmware/spi-master.elf\"", "MASTER")
                .replace("\"firmware/spi-slave.elf\"", "\"firmware/spi-master.elf\"")
                .replace("MASTER", "\"firmware/spi-slave.elf\"")
        });
        run_us(&mut world, 400);
        let m = result(&world, "b", 8);
        assert_eq!(m[7], 1, "master on b finished: {m:x?}");
        assert_eq!(&m[..4], &[0x81, 0x82, 0x83, 0x84]);
        let s = result(&world, "a", 8);
        assert_eq!(&s[..4], &[0xA5, 0x3C, 0x5A, 0xC3]);
        assert_eq!(s[4], 4);
    }

    #[test]
    fn an_8_bit_master_without_frxth_waits_for_16_bits_and_stalls() {
        // CR2.FRXTH left at its reset 0: RXNE needs two bytes in the RX
        // FIFO, so after one 8-bit frame the master polls RXNE forever —
        // what an F4-style driver does on a real G0.
        let mut world = build("env-spi.yaml", |s| {
            s.replace("firmware/spi-master.elf", "firmware/spi-master-nofrxth.elf")
        });
        run_us(&mut world, 400);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 0, "the master never finishes: {a:x?}");
        assert_eq!(a[0], 0, "and never reads DR");
        let reports = world.gpio_net_reports();
        assert_eq!(net(&reports, "sck").edges, 16, "one frame only");
        assert_eq!(result(&world, "b", 8)[4], 1, "the slave got that frame");
    }

    #[test]
    fn frame_sizes_do_not_depend_on_node_order_or_round_size() {
        for ds in [16, 4] {
            assert_order_and_round_independent("env-spi.yaml", env(ds, false), ["a", "b"], 600, 8);
        }
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

/// The nRF52840's SPIM0 / SPIS0 (EasyDMA) against the STM32G071's SPI1, both
/// ways round (`env-spi-nrf52.yaml`, `env-spi-nrf52-slave.yaml`).
mod nrf52_spi {
    use super::*;

    const MASTER_TX: [u8; 4] = [0xA5, 0x3C, 0x5A, 0xC3];
    const SLAVE_TX: [u8; 4] = [0x81, 0x82, 0x83, 0x84];

    /// MISO in mode 0: the answers; at the last trailing edge the next
    /// word's MSB (1: the G071's 0x85, the SPIS's ORC 0xFF); the pull-down
    /// once CS rises and the slave lets go.
    fn miso_edges() -> u64 {
        let mut miso = bits_of(&SLAVE_TX, false);
        miso.push(true);
        miso.push(false);
        edges_of(&miso, false)
    }

    #[test]
    fn an_nrf52_spim_master_exchanges_its_easydma_buffers_with_a_g071_slave() {
        let mut world = build("env-spi-nrf52.yaml", |s| s);
        run_us(&mut world, 600);
        let nrf = result(&world, "nrf", 8);
        assert_eq!(nrf[7], 1, "master finished: {nrf:x?}");
        assert_eq!(&nrf[..4], SLAVE_TX.map(u32::from).as_slice(), "RX buffer");
        assert_eq!((nrf[4], nrf[5]), (4, 4), "TXD.AMOUNT, RXD.AMOUNT");
        assert_eq!(nrf[6], 1, "one END interrupt");
        let b = result(&world, "b", 8);
        assert_eq!(&b[..4], MASTER_TX.map(u32::from).as_slice());
        assert_eq!((b[4], b[5]), (4, 0), "one RXNE per frame, no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 64);
        assert_eq!(net(&reports, "cs").edges, 2);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        assert_eq!(net(&reports, "miso").edges, miso_edges());
    }

    #[test]
    fn an_nrf52_spis_slave_answers_a_g071_master_from_its_easydma_buffer() {
        let mut world = build("env-spi-nrf52-slave.yaml", |s| s);
        run_us(&mut world, 400);
        let a = result(&world, "a", 8);
        assert_eq!(a[7], 1, "master finished: {a:x?}");
        assert_eq!(&a[..4], SLAVE_TX.map(u32::from).as_slice());
        let nrf = result(&world, "nrf", 8);
        assert_eq!(nrf[7], 1, "slave finished: {nrf:x?}");
        assert_eq!(&nrf[..4], MASTER_TX.map(u32::from).as_slice(), "RX buffer");
        assert_eq!((nrf[4], nrf[5]), (4, 4), "RXD.AMOUNT, TXD.AMOUNT");
        assert_eq!(nrf[6], 1, "END_ACQUIRE: the CPU holds the semaphore");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 64);
        assert_eq!(net(&reports, "cs").edges, 2);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        assert_eq!(net(&reports, "miso").edges, miso_edges());
    }

    #[test]
    fn nrf52_spi_exchanges_do_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-spi-nrf52.yaml", |s| s, ["nrf", "b"], 600, 8);
        assert_order_and_round_independent("env-spi-nrf52-slave.yaml", |s| s, ["a", "nrf"], 400, 8);
    }
}

/// The nRF52840's TWIM0 / TWIS0 (EasyDMA) against the STM32G071's modern
/// I²C, both ways round (`env-i2c-nrf52.yaml`, `env-i2c-nrf52-target.yaml`):
/// the same register-file exchange as `env-i2c.yaml`.
mod nrf52_i2c {
    use super::*;

    /// Ten bytes of 9 clocks (address + 3; address + 1, address + 2;
    /// address), one repeated START, three STOPs.
    const SCL_EDGES: u64 = 2 * (9 * 10 + 1 + 3);

    fn check_twim(world: &World) {
        let nrf = result(world, "nrf", 8);
        assert_eq!(nrf[7], 1, "controller finished: {nrf:x?}");
        assert_eq!(nrf[2], 0, "0x42 ACKed the write: ERRORSRC clear");
        assert_eq!(
            &nrf[..2],
            &[0xDE, 0xAD],
            "read back after a repeated START: {nrf:x?} {:x?}",
            result(world, "b", 8)
        );
        assert_eq!(nrf[6], 2, "RXD.AMOUNT");
        assert_eq!(nrf[3], 2, "nobody at 0x50: ERRORSRC.ANACK");
        assert_eq!(nrf[5], 0, "TXD.AMOUNT after an address NACK");
        assert_eq!(nrf[4], 3, "three STOPPED events");

        let b = result(world, "b", 8);
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
    fn an_nrf52_twim_controller_writes_and_reads_a_g071_target() {
        let mut world = build("env-i2c-nrf52.yaml", |s| s);
        run_us(&mut world, 1500);
        check_twim(&world);
    }

    #[test]
    fn a_slow_g071_target_holds_the_nrf52_twim_by_stretching_scl() {
        // The G071 handler waits about 15 us before it answers ADDR or TXIS,
        // several of the TWIM's SCL low periods.
        let mut world = build("env-i2c-nrf52.yaml", |s| {
            s.replace("i2c-target.elf", "i2c-target-slow.elf")
        });
        run_us(&mut world, 2500);
        check_twim(&world);
        // K250: a 256-cycle bit, SCL low for 128 of them.
        let held = longest_low(&world, "nrf", 27);
        assert!(held > 4 * 128, "SCL held low {held} cycles");
    }

    fn check_twis(world: &World) {
        let a = result(world, "a", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?}");
        assert_eq!(a[2], 0, "0x42 ACKed the write");
        assert_eq!(&a[..2], &[0xDE, 0xAD], "read back through a repeated START");
        assert_eq!(a[3], 1, "nobody at 0x50: NACKF");
        assert_eq!(a[4], 3);

        let nrf = result(world, "nrf", 8);
        assert_eq!(nrf[0], 2, "STOPPED for its two transfers only: {nrf:x?}");
        assert_eq!(&nrf[1..3], &[0xDE, 0xAD]);
        assert_eq!(nrf[3], 4, "pointer + 2 data, then the pointer again");
        assert_eq!(nrf[4], 2, "TXD.AMOUNT: two bytes read");
        assert_eq!((nrf[5], nrf[6]), (1, 2), "READ and WRITE events");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let scl = net(&reports, "scl");
        assert!(scl.level && net(&reports, "sda").level);
        assert_eq!(scl.edges, SCL_EDGES, "{scl:#?}");
    }

    #[test]
    fn a_g071_controller_writes_and_reads_an_nrf52_twis_target() {
        let mut world = build("env-i2c-nrf52-target.yaml", |s| s);
        run_us(&mut world, 1500);
        check_twis(&world);
    }

    #[test]
    fn a_slow_nrf52_twis_target_holds_the_controller_by_stretching_scl() {
        // About 25 us before each prepare: the TWIS holds SCL low after its
        // address until the firmware hands it a buffer.
        let mut world = build("env-i2c-nrf52-target.yaml", |s| {
            s.replace("nrf-twis-target.elf", "nrf-twis-target-slow.elf")
        });
        run_us(&mut world, 2500);
        check_twis(&world);
        // The G071's own SCL low period is (SCLL + 1) x 8 = 128 cycles.
        let held = longest_low(&world, "a", 6);
        assert!(held > 4 * 128, "SCL held low {held} cycles");
    }

    #[test]
    fn nrf52_i2c_does_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-i2c-nrf52.yaml", |s| s, ["nrf", "b"], 1500, 8);
        assert_order_and_round_independent(
            "env-i2c-nrf52-target.yaml",
            |s| s,
            ["a", "nrf"],
            1500,
            8,
        );
    }
}

/// The ESP32-C3 and ESP32-C6 GP-SPI2 and I2C0 against the STM32G071, both
/// ways round, routed through the ESP GPIO matrix
/// (`env-spi-esp32c3*.yaml`, `env-i2c-esp32c3*.yaml`; firmware
/// `src/esp32c_*.c`). The C6 runs the same sources built with `-DCHIP_C6` on
/// the same wiring.
mod esp32c3 {
    use super::*;

    const MASTER_TX: [u8; 4] = [0xA5, 0x3C, 0x5A, 0xC3];
    const SLAVE_TX: [u8; 4] = [0x81, 0x82, 0x83, 0x84];
    /// `SPI_DMA_INT_RAW.TRANS_DONE`.
    const TRANS_DONE: u32 = 1 << 12;

    /// The same environment with the ESP32-C6 in place of the C3.
    pub(super) fn c6(yaml: String) -> String {
        yaml.replace("esp32c3-devkit.yaml", "esp32c6-devkitc.yaml")
            .replace("firmware/esp32c3-", "firmware/esp32c6-")
    }

    fn spi_master(chip: fn(String) -> String) {
        let mut world = build("env-spi-esp32c3.yaml", chip);
        run_us(&mut world, 1000);
        let esp = result(&world, "esp", 8);
        assert_eq!(esp[7], 1, "master finished: {esp:x?}");
        assert_eq!(
            &esp[..4],
            SLAVE_TX.map(u32::from).as_slice(),
            "W0 holds what the slave put on MISO"
        );
        assert_ne!(esp[4] & TRANS_DONE, 0, "TRANS_DONE");
        let b = result(&world, "b", 8);
        assert_eq!(&b[..4], MASTER_TX.map(u32::from).as_slice());
        assert_eq!((b[4], b[5]), (4, 0), "one RXNE per byte, no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 64);
        // CS0 frames the whole transaction.
        assert_eq!(net(&reports, "cs").edges, 2);
        assert!(net(&reports, "cs").level, "deselected at the end");
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // MISO: the four answers, then the next queued answer's MSB (0x85)
        // at the last trailing edge, then the pull-down once CS rises.
        assert_eq!(
            net(&reports, "miso").edges,
            data_edges(&SLAVE_TX, false) + 2
        );
    }

    fn spi_slave(chip: fn(String) -> String) {
        let mut world = build("env-spi-esp32c3-slave.yaml", chip);
        run_us(&mut world, 1000);
        let b = result(&world, "b", 8);
        assert_eq!(b[7], 1, "master finished: {b:x?}");
        assert_eq!(&b[..4], SLAVE_TX.map(u32::from).as_slice());
        let esp = result(&world, "esp", 8);
        assert_eq!(esp[7], 1, "slave finished: {esp:x?}");
        assert_eq!(&esp[..4], MASTER_TX.map(u32::from).as_slice());
        assert_eq!(esp[4], 32, "SLAVE1.DATA_BITLEN");
        assert_ne!(esp[5] & TRANS_DONE, 0, "TRANS_DONE when CS rose");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 64);
        assert_eq!(net(&reports, "cs").edges, 2);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // The slave drives MISO only while selected: its four answers, then
        // the pull-down when it lets go.
        let mut miso = bits_of(&SLAVE_TX, false);
        miso.push(false);
        assert_eq!(net(&reports, "miso").edges, edges_of(&miso, false));
    }

    fn i2c_controller(chip: fn(String) -> String, target: &str) {
        let target = target.to_string();
        let mut world = build("env-i2c-esp32c3.yaml", |s| {
            chip(s).replace("firmware/i2c-target.elf", &format!("firmware/{target}"))
        });
        run_us(&mut world, 3000);
        let esp = result(&world, "esp", 8);
        let b = result(&world, "b", 8);
        assert_eq!(esp[7], 1, "controller finished: {esp:x?} target {b:x?}");
        assert_eq!(esp[2], 0, "0x42 ACKed the write");
        assert_eq!(
            &esp[..2],
            &[0xDE, 0xAD],
            "read back through a repeated START"
        );
        assert_eq!(esp[3], 1, "nobody at 0x50: NACK");
        assert_eq!(esp[4], 2, "two transfers ended by TRANS_COMPLETE");

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
        // Address + 3; address + 1, address + 2 (repeated START); address +
        // 1 at 0x50, NACKed after the address.
        assert_eq!(scl.edges, 2 * (9 * 10 + 1 + 3), "{scl:#?}");
    }

    /// The ESP target firmware, the slow one with `slow`.
    fn target_firmware(slow: bool) -> impl Fn(String) -> String {
        move |s: String| {
            if slow {
                s.replace("-i2c-target.elf", "-i2c-target-slow.elf")
            } else {
                s
            }
        }
    }

    fn i2c_target(chip: fn(String) -> String, slow: bool) {
        let mut world = build("env-i2c-esp32c3-target.yaml", |s| {
            target_firmware(slow)(chip(s))
        });
        run_us(&mut world, 3000);
        let a = result(&world, "b", 8);
        let esp = result(&world, "esp", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?} target {esp:x?}");
        assert_eq!(a[2], 0, "0x42 ACKed the write");
        assert_eq!(&a[..2], &[0xDE, 0xAD], "read back through a repeated START");
        assert_eq!(a[3], 1, "nobody at 0x50: NACKF");
        assert_eq!(a[4], 3);

        assert_eq!(
            esp[0], 2,
            "TRANS_COMPLETE for its two transfers only: {esp:x?}"
        );
        assert_eq!(&esp[1..3], &[0xDE, 0xAD]);
        assert_eq!(esp[3], 4, "pointer + 2 data, then the pointer again");
        assert_eq!(esp[4], 2, "one stretch per byte read");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let scl = net(&reports, "scl");
        assert!(scl.level && net(&reports, "sda").level);
        assert_eq!(scl.edges, 2 * (9 * 10 + 1 + 3), "{scl:#?}");
    }

    fn same(yaml: String) -> String {
        yaml
    }

    #[test]
    fn an_esp32c3_master_clocks_a_g071_slave() {
        spi_master(same);
    }

    #[test]
    fn a_g071_master_clocks_an_esp32c3_slave() {
        spi_slave(same);
    }

    #[test]
    fn an_esp32c6_master_clocks_a_g071_slave() {
        spi_master(c6);
    }

    #[test]
    fn a_g071_master_clocks_an_esp32c6_slave() {
        spi_slave(c6);
    }

    #[test]
    fn an_esp32c3_controller_writes_and_reads_a_g071_target() {
        i2c_controller(same, "i2c-target.elf");
    }

    #[test]
    fn an_esp32c6_controller_writes_and_reads_a_g071_target() {
        i2c_controller(c6, "i2c-target.elf");
    }

    #[test]
    fn a_slow_g071_target_holds_the_esp32c3_controller_by_stretching_scl() {
        i2c_controller(same, "i2c-target-slow.elf");
        let mut world = build("env-i2c-esp32c3.yaml", |s| {
            s.replace("firmware/i2c-target.elf", "firmware/i2c-target-slow.elf")
        });
        run_us(&mut world, 3000);
        // The C3's own SCL low period is 200 ticks of 40 MHz, 800 cycles.
        let held = longest_low(&world, "esp", 9);
        assert!(held > 2 * 800, "SCL held low {held} cycles");
    }

    #[test]
    fn a_g071_controller_writes_and_reads_an_esp32c3_target() {
        i2c_target(same, false);
    }

    #[test]
    fn a_g071_controller_writes_and_reads_an_esp32c6_target() {
        i2c_target(c6, false);
    }

    #[test]
    fn a_slow_esp32c3_target_holds_the_g071_controller_by_stretching_scl() {
        i2c_target(same, true);
        let mut world = build("env-i2c-esp32c3-target.yaml", target_firmware(true));
        run_us(&mut world, 3000);
        // The G071's own SCL low period is (SCLL + 1) x 8 = 128 cycles.
        let held = longest_low(&world, "b", 6);
        assert!(held > 4 * 128, "SCL held low {held} cycles");
    }

    /// Run `env` (rewritten by `rewrite`) until the C3's console says
    /// `done` (or `max_ms` pass), and return the console.
    fn esp_hal_run(
        env: &str,
        rewrite: impl Fn(String) -> String,
        done: &str,
        max_ms: u64,
    ) -> (World, String) {
        let mut world = build(env, rewrite);
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        world
            .machines
            .get_mut("esp")
            .unwrap()
            .attach_uart_tx_sink(sink.clone(), false)
            .unwrap();
        let console = || String::from_utf8_lossy(&sink.lock().unwrap()).into_owned();
        let mut t = 0;
        while t < max_ms * 1_000_000_000 && !console().contains(done) {
            t += 100_000_000;
            for (id, r) in world.run_until_ps(t).unwrap() {
                r.unwrap_or_else(|e| panic!("node {id}: {e:?}\n{}", console()));
            }
        }
        let text = console();
        (world, text)
    }

    /// esp-hal 1.1's own blocking SPI master driver (`esp32c3-esp-hal/`),
    /// hardware CS0, against the G071 slave.
    #[test]
    fn an_esp_hal_spi_master_on_an_esp32c3_clocks_a_g071_slave() {
        let (world, console) = esp_hal_run(
            "env-spi-esp32c3.yaml",
            |s| s.replace("esp32c3-spi-master.elf", "esp32c3-esp-hal-spi-master.elf"),
            "C3 SPI DONE",
            200,
        );
        assert!(console.contains("C3 spi true 81 82 83 84"), "{console}");
        let b = result(&world, "b", 8);
        assert_eq!(&b[..4], MASTER_TX.map(u32::from).as_slice(), "{console}");
        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 64);
        // esp-hal turns the CS pad's output stage on before it routes FSPICS0
        // to it, so the pad first drives its GPIO latch (0) and then the idle
        // CS (1): one low pulse at configuration, as on silicon, then the
        // transaction's.
        assert_eq!(net(&reports, "cs").edges, 4);
        assert!(net(&reports, "cs").level);
    }

    /// esp-hal 1.1's blocking I2C master driver against the G071 target, a
    /// NACK at 0x50 included, and against the slow target that stretches.
    #[test]
    fn an_esp_hal_i2c_master_on_an_esp32c3_writes_and_reads_a_g071_target() {
        for target in ["i2c-target.elf", "i2c-target-slow.elf"] {
            let (world, console) = esp_hal_run(
                "env-i2c-esp32c3.yaml",
                |s| {
                    s.replace(
                        "esp32c3-i2c-controller.elf",
                        "esp32c3-esp-hal-i2c-master.elf",
                    )
                    .replace("firmware/i2c-target.elf", &format!("firmware/{target}"))
                },
                "C3 I2C DONE",
                200,
            );
            assert!(console.contains("C3 write true"), "{target}: {console}");
            assert!(
                console.contains("C3 read true DE AD"),
                "{target}: {console}"
            );
            assert!(
                console.contains("C3 nack Err(AcknowledgeCheckFailed("),
                "{target}: {console}"
            );
            let b = result(&world, "b", 8);
            assert_eq!(b[0], 2, "{target}: two transfers to 0x42: {b:x?}");
            assert_eq!(&b[1..3], &[0xDE, 0xAD]);
            let reports = world.gpio_net_reports();
            no_contention(&reports);
            assert!(net(&reports, "scl").level && net(&reports, "sda").level);
        }
    }

    #[test]
    fn esp32c3_buses_do_not_depend_on_node_order_or_round_size() {
        for env in ["env-spi-esp32c3.yaml", "env-spi-esp32c3-slave.yaml"] {
            assert_order_and_round_independent(env, |s| s, ["esp", "b"], 1000, 8);
        }
        for env in ["env-i2c-esp32c3.yaml", "env-i2c-esp32c3-target.yaml"] {
            assert_order_and_round_independent(env, |s| s, ["esp", "b"], 3000, 8);
        }
    }

    #[test]
    fn esp32c6_buses_do_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-spi-esp32c3.yaml", c6, ["esp", "b"], 1000, 8);
        assert_order_and_round_independent(
            "env-i2c-esp32c3-target.yaml",
            c6,
            ["esp", "b"],
            3000,
            8,
        );
    }
}

/// Wire history of `pin` of `node` as the net delivered it there: (world
/// time in ps, level), in time order.
fn deliveries(world: &World, node: &str, pin: u8) -> Vec<(u64, bool)> {
    let mut out: Vec<_> = world
        .gpio_net_applied()
        .iter()
        .filter(|d| d.node == node && d.pin == pin)
        .map(|d| (d.due_ps, d.level))
        .collect();
    out.sort();
    out
}

/// Hardware NSS on the FIFO SPI of the STM32L476 and STM32G474
/// (`env-spi-l4-nss.yaml`, the G474 swapped in through `system-g474.yaml`):
/// the master's SPI drives PA4 = SPI1_NSS (AF5, CR2.SSOE=1) and the slave
/// (SSM=0) is selected by the level its own PA4 = SPI1_NSS pad reads off the
/// net. `l4-spi-nss-master-nssp.elf` adds CR2.NSSP, NSS pulse mode.
mod l4_g4_spi_nss {
    use super::*;

    const MASTER_TX: [u8; 4] = [0xA5, 0x3C, 0x5A, 0xC3];
    const SLAVE_TX: [u8; 4] = [0x81, 0x82, 0x83, 0x84];

    fn g474(s: String) -> String {
        s.replace("system-l476.yaml", "system-g474.yaml")
    }

    fn nssp(s: String) -> String {
        s.replace("l4-spi-nss-master.elf", "l4-spi-nss-master-nssp.elf")
    }

    /// The register-file results both pairs leave, whatever the NSS mode.
    fn check_exchange(world: &World, part: &str) {
        let a = result(world, "a", 8);
        assert_eq!(a[7], 1, "{part}: master finished: {a:x?}");
        assert_eq!(&a[..4], SLAVE_TX.map(u32::from).as_slice(), "{part}");
        // SR after SPE=0: TXE only, both FIFOs empty, no OVR/MODF.
        assert_eq!(a[4], 0x0002, "{part}: master SR");
        let b = result(world, "b", 8);
        assert_eq!(&b[..4], MASTER_TX.map(u32::from).as_slice(), "{part}");
        assert_eq!((b[4], b[5]), (4, 0), "{part}: one RXNE per frame, no OVR");
    }

    fn ssoe_drives_nss(rewrite: fn(String) -> String, part: &str) {
        let mut world = build("env-spi-l4-nss.yaml", rewrite);
        run_us(&mut world, 400);
        check_exchange(&world, part);

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // NSS: driven low by SPE=1 with SSOE, released at SPE=0 (pull-up).
        assert_eq!(net(&reports, "nss").edges, 2, "{part}");
        assert!(net(&reports, "nss").level, "{part}: released at the end");
        assert_eq!(net(&reports, "sck").edges, 64, "{part}");
        // MOSI: the bytes, then the pull-down once SPE=0 stops the master
        // driving it (0xC3 left it high).
        let mut mosi = bits_of(&MASTER_TX, false);
        mosi.push(false);
        assert_eq!(
            net(&reports, "mosi").edges,
            edges_of(&mosi, false),
            "{part}"
        );
        // MISO: the answers, the MSB (1) of the next one (0x85, queued from
        // the RXNE interrupt) at the last trailing edge, then the pull-down
        // once NSS rises and the slave lets go.
        let mut miso = bits_of(&SLAVE_TX, false);
        miso.push(true);
        miso.push(false);
        assert_eq!(
            net(&reports, "miso").edges,
            edges_of(&miso, false),
            "{part}"
        );
        assert!(!net(&reports, "miso").level, "{part}");
    }

    #[test]
    fn an_l476_masters_ssoe_drives_nss_and_the_slave_follows_its_nss_pad() {
        ssoe_drives_nss(|s| s, "L476");
    }

    #[test]
    fn a_g474_masters_ssoe_drives_nss_and_the_slave_follows_its_nss_pad() {
        ssoe_drives_nss(g474, "G474");
    }

    fn pulse_mode(rewrite: fn(String) -> String, part: &str) {
        let mut world = build("env-spi-l4-nss.yaml", rewrite);
        run_us(&mut world, 400);
        check_exchange(&world, part);

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // NSSP with a master that polls RXNE between frames: NSS falls at
        // SPE=1, rises after each frame and falls again as the next starts:
        // 1 + 4 + 3 edges, high at the end (and SPE=0 leaves it to the
        // pull-up).
        assert_eq!(net(&reports, "nss").edges, 8, "{part}");
        assert!(net(&reports, "nss").level, "{part}");
        assert_eq!(net(&reports, "sck").edges, 64, "{part}");
        // Every NSS low period holds exactly one frame: 16 SCK edges
        // delivered to the slave between each fall and the next rise.
        let nss = deliveries(&world, "b", 4);
        let sck = deliveries(&world, "b", 5);
        assert_eq!(nss.len(), 8, "{part}: {nss:?}");
        for pair in nss.chunks(2) {
            let (fall, rise) = (pair[0], pair[1]);
            assert!(!fall.1 && rise.1, "{part}: {nss:?}");
            let inside = sck
                .iter()
                .filter(|&&(t, _)| t > fall.0 && t <= rise.0)
                .count();
            assert_eq!(inside, 16, "{part}: SCK edges in one NSS low period");
        }
    }

    #[test]
    fn an_l476_master_in_nss_pulse_mode_frames_every_byte() {
        pulse_mode(nssp, "L476");
    }

    #[test]
    fn a_g474_master_in_nss_pulse_mode_frames_every_byte() {
        pulse_mode(|s| g474(nssp(s)), "G474");
    }

    /// Drop the `nss` net: the slave's NSS pad reads no level.
    fn without_nss(s: String) -> String {
        let start = s.find("  - type: gpio_net").unwrap();
        let end = start + 1 + s[start + 1..].find("  - type: gpio_net").unwrap();
        format!("{}{}", &s[..start], &s[end..])
    }

    #[test]
    fn with_nss_unwired_the_hardware_select_slave_is_never_selected() {
        for (rewrite, part) in [
            (without_nss as fn(String) -> String, "L476"),
            (|s| g474(without_nss(s)), "G474"),
        ] {
            let mut world = build("env-spi-l4-nss.yaml", rewrite);
            run_us(&mut world, 400);
            let a = result(&world, "a", 8);
            assert_eq!(a[7], 1, "{part}: master finished: {a:x?}");
            assert_eq!(&a[..4], &[0, 0, 0, 0], "{part}: MISO is the pull-down");
            let b = result(&world, "b", 8);
            assert_eq!(b[4], 0, "{part}: the slave was never selected");
            let reports = world.gpio_net_reports();
            no_contention(&reports);
            assert_eq!(net(&reports, "miso").edges, 0, "{part}");
        }
    }

    #[test]
    fn hardware_nss_frames_do_not_depend_on_node_order_or_round_size() {
        for rewrite in [(|s| s) as fn(String) -> String, g474, nssp, |s| {
            g474(nssp(s))
        }] {
            assert_order_and_round_independent("env-spi-l4-nss.yaml", rewrite, ["a", "b"], 400, 8);
        }
    }
}

/// The nRF52840's SPIM3 with its hardware chip select and D/CX against the
/// STM32G071's SPI1 on hardware NSS (`env-spi-nrf52-spim3.yaml`): CSN on
/// P0.02 (`CSNPOL`, `IFTIMING.CSNDUR` = 64 cycles), D/CX on P0.06 with
/// `DCXCNT` = 1.
mod nrf52_spim3 {
    use super::*;

    const MASTER_TX: [u8; 4] = [0xA5, 0x3C, 0x5A, 0xC3];
    const SLAVE_TX: [u8; 4] = [0x81, 0x82, 0x83, 0x84];
    /// One cycle of the nRF52840's 64 MHz clock, in ps.
    const CYCLE_PS: u64 = 15_625;
    /// `IFTIMING.CSNDUR` as the firmware sets it.
    const CSNDUR: u64 = 64;
    /// Half an SCK period at K125: 512 cycles a bit.
    const HALF: u64 = 256;

    fn csnpol_high(s: String) -> String {
        s.replace("nrf-spim3-master.elf", "nrf-spim3-master-csnpol.elf")
    }

    /// CSN against SCK and D/CX against the bytes, as the G071 sees them.
    /// `active` is CSN's asserted level.
    fn check_timing(world: &World, active: bool) {
        let cs = deliveries(world, "b", 4);
        let sck = deliveries(world, "b", 5);
        let dcx = deliveries(world, "b", 8);
        let asserted = cs
            .iter()
            .find(|&&(_, l)| l == active)
            .expect("CSN asserted")
            .0;
        let released = cs
            .iter()
            .rev()
            .find(|&&(_, l)| l != active)
            .expect("CSN released")
            .0;
        assert_eq!(sck.len(), 64, "{sck:?}");
        // CSNDUR, then half an SCK period with the first bit on MOSI, then
        // the first edge; and CSNDUR from the last edge to the release.
        assert_eq!(sck[0].0 - asserted, (CSNDUR + HALF) * CYCLE_PS);
        assert_eq!(released - sck[63].0, CSNDUR * CYCLE_PS);
        // D/CX: low with CSN for the one command byte, high again at that
        // byte's last SCK edge, and nothing more.
        assert_eq!(dcx, vec![(asserted, false), (sck[15].0, true)]);
    }

    #[test]
    fn spim3_drives_its_own_csn_and_dcx_around_the_exchange() {
        let mut world = build("env-spi-nrf52-spim3.yaml", |s| s);
        run_us(&mut world, 600);
        let nrf = result(&world, "nrf", 8);
        assert_eq!(nrf[7], 1, "master finished: {nrf:x?}");
        assert_eq!(&nrf[..4], SLAVE_TX.map(u32::from).as_slice(), "RX buffer");
        assert_eq!((nrf[4], nrf[5]), (4, 4), "TXD.AMOUNT, RXD.AMOUNT");
        assert_eq!(nrf[6], 1, "one END interrupt");
        let b = result(&world, "b", 8);
        assert_eq!(&b[..4], MASTER_TX.map(u32::from).as_slice());
        assert_eq!((b[4], b[5]), (4, 0), "one RXNE per frame, no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // CSNPOL = 0: idle high from ENABLE (the pull-up's level, no edge),
        // low for the transfer.
        assert_eq!(net(&reports, "cs").edges, 2);
        assert!(net(&reports, "cs").level);
        assert_eq!(net(&reports, "sck").edges, 64);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // MISO: the answers; at the last trailing edge the next word's MSB
        // (1: the G071's 0x85); the pull-down once CSN rises.
        let mut miso = bits_of(&SLAVE_TX, false);
        miso.push(true);
        miso.push(false);
        assert_eq!(net(&reports, "miso").edges, edges_of(&miso, false));
        assert_eq!(net(&reports, "dcx").edges, 2);
        assert!(net(&reports, "dcx").level);
        check_timing(&world, false);
    }

    #[test]
    fn an_active_high_csn_selects_the_active_low_slave_only_while_idle() {
        let mut world = build("env-spi-nrf52-spim3.yaml", csnpol_high);
        run_us(&mut world, 600);
        let nrf = result(&world, "nrf", 8);
        assert_eq!(nrf[7], 1, "master finished: {nrf:x?}");
        // The G071 is deselected exactly while the SPIM clocks: MISO is the
        // pull-down and the slave takes no frame.
        assert_eq!(&nrf[..4], &[0, 0, 0, 0]);
        assert_eq!((nrf[4], nrf[5]), (4, 4));
        assert_eq!(result(&world, "b", 8)[4], 0);

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // The GPIO latch takes the pad low first (inactive for CSNPOL = 1),
        // ENABLE keeps it there, the transfer raises and drops it.
        assert_eq!(net(&reports, "cs").edges, 3);
        assert!(!net(&reports, "cs").level);
        assert_eq!(net(&reports, "sck").edges, 64);
        assert_eq!(net(&reports, "dcx").edges, 2);
        check_timing(&world, true);
    }

    #[test]
    fn with_csn_unwired_the_g071_is_never_selected() {
        // Without the `cs` net the G071's NSS pad reads no level.
        let mut world = build("env-spi-nrf52-spim3.yaml", |s| {
            let start = s.find("  - type: gpio_net").unwrap();
            let end = start + 1 + s[start + 1..].find("  - type: gpio_net").unwrap();
            format!("{}{}", &s[..start], &s[end..])
        });
        run_us(&mut world, 600);
        let nrf = result(&world, "nrf", 8);
        assert_eq!(nrf[7], 1, "master finished: {nrf:x?}");
        assert_eq!(&nrf[..4], &[0, 0, 0, 0], "MISO is the pull-down");
        assert_eq!(result(&world, "b", 8)[4], 0, "never selected");
    }

    #[test]
    fn spim3_exchanges_do_not_depend_on_node_order_or_round_size() {
        assert_order_and_round_independent("env-spi-nrf52-spim3.yaml", |s| s, ["nrf", "b"], 600, 8);
        assert_order_and_round_independent(
            "env-spi-nrf52-spim3.yaml",
            csnpol_high,
            ["nrf", "b"],
            600,
            8,
        );
    }
}

/// The ESP32-S3 GP-SPI2 and I2C0, and the classic ESP32 VSPI (master), HSPI
/// (slave) and I2C0, against the STM32G071, both ways round, routed through
/// each part's GPIO matrix (`env-spi-esp32s3*.yaml`, `env-spi-esp32*.yaml`,
/// `env-i2c-...`; firmware `src/esp32x_*.S`, hand-written Xtensa assembly
/// of the `esp32c_*.c` sequences). The same, on bank-1 pads (GPIO32 and up),
/// for the second controller of a kind: the S3's GP-SPI3 and I2C1 and the
/// classic part's I2C1, and the classic VSPI / HSPI with the input-only
/// GPIO34..36 taking lines in. Plain GPIO on bank-1 pads between the two
/// Xtensa parts.
mod esp32_xtensa {
    use super::*;

    const MASTER_TX: [u8; 4] = [0xA5, 0x3C, 0x5A, 0xC3];
    const SLAVE_TX: [u8; 4] = [0x81, 0x82, 0x83, 0x84];

    /// An Xtensa part: its env-file name and the flag word bit that says a
    /// transaction is done (`SPI_DMA_INT_RAW.TRANS_DONE` on the S3,
    /// `SPI_SLAVE.TRANS_DONE` on the classic part).
    #[derive(Clone, Copy)]
    struct Chip {
        name: &'static str,
        trans_done: u32,
        /// The part's I2C pads: (SDA, SCL).
        i2c_pads: (u8, u8),
        /// What tells the env files of this variant apart: `(spi, i2c)`.
        variant: (&'static str, &'static str),
    }

    const S3: Chip = Chip {
        name: "esp32s3",
        trans_done: 1 << 12,
        i2c_pads: (8, 9),
        variant: ("", ""),
    };
    const CLASSIC: Chip = Chip {
        name: "esp32",
        trans_done: 1 << 4,
        i2c_pads: (21, 22),
        variant: ("", ""),
    };
    /// The S3's GP-SPI3 (GPIO34..37) and I2C1 (GPIO41 SDA, GPIO42 SCL), on
    /// bank-1 pads.
    const S3_HI: Chip = Chip {
        name: "esp32s3",
        trans_done: 1 << 12,
        i2c_pads: (41, 42),
        variant: ("-spi3", "-i2c1"),
    };
    /// The classic part's VSPI master / HSPI slave on bank-1 pads (the
    /// input-only GPIO34..36 taking lines in), and its I2C1 on GPIO32 (SDA)
    /// / GPIO33 (SCL).
    const CLASSIC_HI: Chip = Chip {
        name: "esp32",
        trans_done: 1 << 4,
        i2c_pads: (32, 33),
        variant: ("-hi", "-i2c1"),
    };

    fn env(kind: &str, chip: Chip, suffix: &str) -> String {
        let variant = if kind == "spi" {
            chip.variant.0
        } else {
            chip.variant.1
        };
        format!("env-{kind}-{}{variant}{suffix}.yaml", chip.name)
    }

    fn spi_master(chip: Chip) {
        let mut world = build(&env("spi", chip, ""), |s| s);
        run_us(&mut world, 1000);
        let esp = result(&world, "esp", 8);
        assert_eq!(esp[7], 1, "master finished: {esp:x?}");
        assert_eq!(
            &esp[..4],
            SLAVE_TX.map(u32::from).as_slice(),
            "W0 holds what the slave put on MISO"
        );
        assert_ne!(esp[4] & chip.trans_done, 0, "TRANS_DONE: {esp:x?}");
        let b = result(&world, "b", 8);
        assert_eq!(&b[..4], MASTER_TX.map(u32::from).as_slice());
        assert_eq!((b[4], b[5]), (4, 0), "one RXNE per byte, no OVR");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 64);
        // CS0 frames the whole transaction.
        assert_eq!(net(&reports, "cs").edges, 2);
        assert!(net(&reports, "cs").level, "deselected at the end");
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // MISO: the four answers, then the next queued answer's MSB (0x85)
        // at the last trailing edge, then the pull-down once CS rises.
        assert_eq!(
            net(&reports, "miso").edges,
            data_edges(&SLAVE_TX, false) + 2
        );
    }

    fn spi_slave(chip: Chip) {
        let mut world = build(&env("spi", chip, "-slave"), |s| s);
        run_us(&mut world, 1000);
        let b = result(&world, "b", 8);
        assert_eq!(b[7], 1, "master finished: {b:x?}");
        assert_eq!(&b[..4], SLAVE_TX.map(u32::from).as_slice());
        let esp = result(&world, "esp", 8);
        assert_eq!(esp[7], 1, "slave finished: {esp:x?}");
        assert_eq!(&esp[..4], MASTER_TX.map(u32::from).as_slice());
        assert_eq!(esp[4], 32, "bits received");
        assert_ne!(esp[5] & chip.trans_done, 0, "TRANS_DONE when CS rose");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 64);
        assert_eq!(net(&reports, "cs").edges, 2);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // The slave drives MISO only while selected: its four answers, then
        // the pull-down when it lets go.
        let mut miso = bits_of(&SLAVE_TX, false);
        miso.push(false);
        assert_eq!(net(&reports, "miso").edges, edges_of(&miso, false));
    }

    fn i2c_controller(chip: Chip, target: &str) -> World {
        let target = target.to_string();
        let mut world = build(&env("i2c", chip, ""), |s| {
            s.replace("firmware/i2c-target.elf", &format!("firmware/{target}"))
        });
        run_us(&mut world, 3000);
        let esp = result(&world, "esp", 8);
        let b = result(&world, "b", 8);
        assert_eq!(esp[7], 1, "controller finished: {esp:x?} target {b:x?}");
        assert_eq!(esp[2], 0, "0x42 ACKed the write");
        assert_eq!(
            &esp[..2],
            &[0xDE, 0xAD],
            "read back through a repeated START"
        );
        assert_eq!(esp[3], 1, "nobody at 0x50: NACK");
        assert_eq!(esp[4], 2, "two transfers ended by TRANS_COMPLETE");

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
        // Address + 3; address + 1, address + 2 (repeated START); address +
        // 1 at 0x50, NACKed after the address.
        assert_eq!(scl.edges, 2 * (9 * 10 + 1 + 3), "{scl:#?}");
        world
    }

    fn i2c_target(chip: Chip, slow: bool) -> World {
        let mut world = build(&env("i2c", chip, "-target"), |s| {
            if slow {
                s.replace("-target.elf", "-target-slow.elf")
            } else {
                s
            }
        });
        run_us(&mut world, 3000);
        let a = result(&world, "b", 8);
        let esp = result(&world, "esp", 8);
        assert_eq!(a[7], 1, "controller finished: {a:x?} target {esp:x?}");
        assert_eq!(a[2], 0, "0x42 ACKed the write");
        assert_eq!(&a[..2], &[0xDE, 0xAD], "read back through a repeated START");
        assert_eq!(a[3], 1, "nobody at 0x50: NACKF");
        assert_eq!(a[4], 3);

        assert_eq!(
            esp[0], 2,
            "TRANS_COMPLETE for its two transfers only: {esp:x?}"
        );
        assert_eq!(&esp[1..3], &[0xDE, 0xAD]);
        assert_eq!(esp[3], 4, "pointer + 2 data, then the pointer again");
        assert_eq!(esp[4], 2, "two bytes read from it");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        let scl = net(&reports, "scl");
        assert!(scl.level && net(&reports, "sda").level);
        assert_eq!(scl.edges, 2 * (9 * 10 + 1 + 3), "{scl:#?}");
        world
    }

    #[test]
    fn an_esp32s3_master_clocks_a_g071_slave() {
        spi_master(S3);
    }

    #[test]
    fn a_g071_master_clocks_an_esp32s3_slave() {
        spi_slave(S3);
    }

    #[test]
    fn a_classic_esp32_vspi_master_clocks_a_g071_slave() {
        spi_master(CLASSIC);
    }

    #[test]
    fn a_g071_master_clocks_a_classic_esp32_hspi_slave() {
        spi_slave(CLASSIC);
    }

    #[test]
    fn an_esp32s3_controller_writes_and_reads_a_g071_target() {
        i2c_controller(S3, "i2c-target.elf");
    }

    #[test]
    fn a_classic_esp32_controller_writes_and_reads_a_g071_target() {
        i2c_controller(CLASSIC, "i2c-target.elf");
    }

    /// The G071 slow target stretches SCL; each controller's own SCL low
    /// period is 1200 CPU cycles (S3: 200 ticks of 40 MHz at 240 MHz;
    /// classic: 400 APB cycles of 80 MHz).
    #[test]
    fn a_slow_g071_target_holds_the_xtensa_controllers_by_stretching_scl() {
        for chip in [S3, CLASSIC, S3_HI, CLASSIC_HI] {
            let world = i2c_controller(chip, "i2c-target-slow.elf");
            let held = longest_low(&world, "esp", chip.i2c_pads.1);
            assert!(held > 2 * 1200, "{}: SCL held low {held} cycles", chip.name);
        }
    }

    #[test]
    fn a_g071_controller_writes_and_reads_an_esp32s3_target() {
        i2c_target(S3, false);
    }

    #[test]
    fn a_g071_controller_writes_and_reads_a_classic_esp32_target() {
        i2c_target(CLASSIC, false);
    }

    #[test]
    fn a_slow_esp32s3_target_holds_the_g071_controller_by_stretching_scl() {
        let world = i2c_target(S3, true);
        // The G071's own SCL low period is (SCLL + 1) x 8 = 128 cycles.
        let held = longest_low(&world, "b", 6);
        assert!(held > 4 * 128, "SCL held low {held} cycles");
    }

    #[test]
    fn xtensa_buses_do_not_depend_on_node_order_or_round_size() {
        for chip in [S3, CLASSIC] {
            for env_file in [env("spi", chip, ""), env("spi", chip, "-slave")] {
                assert_order_and_round_independent(&env_file, |s| s, ["esp", "b"], 1000, 8);
            }
            for env_file in [env("i2c", chip, ""), env("i2c", chip, "-target")] {
                assert_order_and_round_independent(&env_file, |s| s, ["esp", "b"], 3000, 8);
            }
        }
    }

    // ── second instances, bank-1 pads ──────────────────────────────────────

    #[test]
    fn an_esp32s3_spi3_master_on_bank_one_pads_clocks_a_g071_slave() {
        spi_master(S3_HI);
    }

    #[test]
    fn a_g071_master_clocks_an_esp32s3_spi3_slave_on_bank_one_pads() {
        spi_slave(S3_HI);
    }

    #[test]
    fn a_classic_esp32_vspi_master_on_bank_one_pads_clocks_a_g071_slave() {
        // MISO comes in on GPIO34, which has no output stage at all.
        spi_master(CLASSIC_HI);
    }

    #[test]
    fn a_g071_master_clocks_a_classic_esp32_hspi_slave_through_input_only_pads() {
        spi_slave(CLASSIC_HI);
    }

    #[test]
    fn an_esp32s3_i2c1_controller_on_bank_one_pads_writes_and_reads_a_g071_target() {
        i2c_controller(S3_HI, "i2c-target.elf");
    }

    #[test]
    fn a_classic_esp32_i2c1_controller_on_gpio32_33_writes_and_reads_a_g071_target() {
        i2c_controller(CLASSIC_HI, "i2c-target.elf");
    }

    #[test]
    fn a_g071_controller_writes_and_reads_an_esp32s3_i2c1_target_on_bank_one_pads() {
        i2c_target(S3_HI, false);
    }

    #[test]
    fn a_g071_controller_writes_and_reads_a_classic_esp32_i2c1_target_on_gpio32_33() {
        i2c_target(CLASSIC_HI, false);
    }

    #[test]
    fn a_slow_esp32s3_i2c1_target_holds_the_g071_controller_by_stretching_scl() {
        let world = i2c_target(S3_HI, true);
        let held = longest_low(&world, "b", 6);
        assert!(held > 4 * 128, "SCL held low {held} cycles");
    }

    #[test]
    fn bank_one_buses_do_not_depend_on_node_order_or_round_size() {
        for chip in [S3_HI, CLASSIC_HI] {
            for env_file in [env("spi", chip, ""), env("spi", chip, "-slave")] {
                assert_order_and_round_independent(&env_file, |s| s, ["esp", "b"], 1000, 8);
            }
            for env_file in [env("i2c", chip, ""), env("i2c", chip, "-target")] {
                assert_order_and_round_independent(&env_file, |s| s, ["esp", "b"], 3000, 8);
            }
        }
    }

    /// Plain GPIO on bank-1 pads, classic ESP32 against ESP32-S3
    /// (`env-gpio-esp32-esp32s3.yaml`, `src/esp32x_gpio_hi.S`): OUT1 /
    /// ENABLE1 drive the pad onto the net, the other part reads it in IN1
    /// and latches each edge in STATUS1 — the classic part on its
    /// input-only GPIO39.
    #[test]
    fn bank_one_gpio_pads_hand_a_level_between_a_classic_esp32_and_an_esp32s3() {
        const ENV: &str = "env-gpio-esp32-esp32s3.yaml";
        let mut world = build(ENV, |s| s);
        run_us(&mut world, 400);
        for node in ["esp", "esps3"] {
            let r = result(&world, node, 8);
            assert_eq!(r[7], 1, "{node} finished: {r:x?}");
            assert_eq!(r[0], 4, "{node}: four rounds");
            assert_eq!(r[1], 8, "{node}: every edge latched STATUS1: {r:x?}");
            assert_eq!(r[2], 0, "{node}: STATUS1 clear when armed");
        }
        let reports = world.gpio_net_reports();
        no_contention(&reports);
        for name in ["a2b", "b2a"] {
            let n = net(&reports, name);
            assert_eq!(n.edges, 8, "{n:#?}");
            assert!(!n.level, "{name} low at the end");
        }
        assert_order_and_round_independent(ENV, |s| s, ["esp", "esps3"], 400, 8);
    }

    /// RTC GPIO across nets, classic ESP32 against ESP32-S3
    /// (`env-rtc-gpio-esp32-esp32s3.yaml`, `src/esp32x_rtc_gpio.S`): pads the
    /// RTC IO MUX owns (`MUX_SEL` = 1) driven by `RTC_GPIO_OUT` / `ENABLE`
    /// and read in `RTC_GPIO_IN`, every edge latched in `RTC_GPIO_STATUS` by
    /// `RTC_GPIO_PINn.INT_TYPE`; the classic open drain released to the S3's
    /// `RUE` pull-up. Then the hand-off: a digital output on GPIO27 that the
    /// RTC IO MUX owns does not reach the net until `MUX_SEL` clears.
    #[test]
    fn rtc_gpio_pads_hand_a_level_between_a_classic_esp32_and_an_esp32s3() {
        const ENV: &str = "env-rtc-gpio-esp32-esp32s3.yaml";
        let mut world = build(ENV, |s| s);
        run_us(&mut world, 400);
        let esp = result(&world, "esp", 8);
        let s3 = result(&world, "esps3", 8);
        for (node, r) in [("esp", &esp), ("esps3", &s3)] {
            assert_eq!(r[7], 1, "{node} finished: {r:x?}");
            assert_eq!(r[0], 4, "{node}: four rounds");
            // Four rounds and the hand-off round: ten waits, each on an
            // edge its RTC_GPIO_STATUS bit latched.
            assert_eq!(r[1], 10, "{node}: every edge latched: {r:x?}");
            assert_eq!(r[2], 0, "{node}: STATUS clear when armed");
        }
        // The classic part: GPIO27 reads 0 in GPIO_IN while the RTC IO MUX
        // owns it (TOUCH_PAD7.FUN_IE clear), its own driven 1 once digital.
        assert_eq!((esp[4], esp[3]), (0, 1), "classic GPIO_IN bit 27: {esp:x?}");
        // The S3 sampled the hand-off net low, then high, and its GPIO6
        // latched the rising edge (INT_TYPE 1).
        assert_eq!((s3[3], s3[4], s3[5]), (0, 1, 1), "S3 hand-off: {s3:x?}");

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        // a2b: high on the S3's reset pull-up, low once the classic open
        // drain holds 0, then a release and a drive per round (5 rounds).
        // b2a: the same, from the S3's digital pull-up to its RTC output.
        for name in ["a2b", "b2a"] {
            let n = net(&reports, name);
            assert_eq!(n.edges, 1 + 2 * 5, "{n:#?}");
            assert!(!n.level, "{name} low at the end");
        }
        // handoff: the S3's reset pull-up, its RDE, then the classic digital
        // 1 once MUX_SEL cleared: the pad never carried it before.
        let h = net(&reports, "handoff");
        assert_eq!(h.edges, 2, "{h:#?}");
        assert!(h.level, "the digital output carried at the end");
        assert_order_and_round_independent(ENV, |s| s, ["esp", "esps3"], 400, 8);
    }
}

/// The ESP32-C3 / C6 GP-SPI2 moving its data by GDMA (`src/esp32c_spi_dma_*.c`)
/// against a G071 moving its own by DMA (`src/spi_dma_*.c`), both roles.
mod esp32c3_dma {
    use super::esp32c3::c6;
    use super::spi_dma::{ISR_BOTH_DONE, MASTER_TX, SLAVE_TX};
    use super::*;

    /// `SPI_DMA_INT_RAW.TRANS_DONE`.
    const TRANS_DONE: u32 = 1 << 12;
    /// The IN descriptor after the transfer: back with the CPU (owner 0),
    /// `suc_eof`, 8 bytes received into its 8-byte buffer.
    const IN_DESC_DONE: u32 = (1 << 30) | (8 << 12) | 8;

    fn same(yaml: String) -> String {
        yaml
    }

    fn spi_dma_master(chip: fn(String) -> String) {
        let mut world = build("env-spi-esp32c3-dma.yaml", chip);
        run_us(&mut world, 1500);
        let esp = result(&world, "esp", 16);
        assert_eq!(esp[15], 1, "master finished: {esp:x?}");
        assert_eq!(
            &esp[..8],
            SLAVE_TX.map(u32::from).as_slice(),
            "the IN descriptor's buffer holds what the slave put on MISO"
        );
        assert_ne!(esp[8] & TRANS_DONE, 0, "TRANS_DONE");
        assert_eq!(esp[9], IN_DESC_DONE, "IN descriptor written back");
        let b = result(&world, "b", 16);
        assert_eq!(b[15], 1, "slave finished: {b:x?}");
        assert_eq!(&b[..8], MASTER_TX.map(u32::from).as_slice());
        assert_eq!(b[8], ISR_BOTH_DONE);
        assert_eq!((b[9], b[10]), (0, 0));

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 128);
        // CS0 frames the whole transaction.
        assert_eq!(net(&reports, "cs").edges, 2);
        assert!(net(&reports, "cs").level, "deselected at the end");
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // The G071's eight answers; its FIFO is then empty and it shifts the
        // MSB of the last byte it received (0x7E: 0), then the pull-down.
        assert_eq!(net(&reports, "miso").edges, data_edges(&SLAVE_TX, false));
    }

    fn spi_dma_slave(chip: fn(String) -> String) {
        let mut world = build("env-spi-esp32c3-dma-slave.yaml", chip);
        run_us(&mut world, 1500);
        let b = result(&world, "b", 16);
        assert_eq!(b[15], 1, "master finished: {b:x?}");
        assert_eq!(&b[..8], SLAVE_TX.map(u32::from).as_slice());
        assert_eq!(b[8], ISR_BOTH_DONE);
        let esp = result(&world, "esp", 16);
        assert_eq!(esp[15], 1, "slave finished: {esp:x?}");
        assert_eq!(&esp[..8], MASTER_TX.map(u32::from).as_slice());
        assert_eq!(esp[8], 64, "SLAVE1.DATA_BITLEN");
        assert_ne!(esp[9] & TRANS_DONE, 0, "TRANS_DONE when CS rose");
        assert_eq!(esp[10], IN_DESC_DONE);

        let reports = world.gpio_net_reports();
        no_contention(&reports);
        assert_eq!(net(&reports, "sck").edges, 128);
        assert_eq!(net(&reports, "cs").edges, 2);
        assert_eq!(net(&reports, "mosi").edges, data_edges(&MASTER_TX, false));
        // The slave drives MISO only while selected: its eight answers (0x88
        // ends in 0), then the pull-down when it lets go.
        assert_eq!(net(&reports, "miso").edges, data_edges(&SLAVE_TX, false));
    }

    #[test]
    fn an_esp32c3_gdma_master_exchanges_a_buffer_with_a_g071_dma_slave() {
        spi_dma_master(same);
    }

    #[test]
    fn a_g071_dma_master_exchanges_a_buffer_with_an_esp32c3_gdma_slave() {
        spi_dma_slave(same);
    }

    #[test]
    fn an_esp32c6_gdma_master_exchanges_a_buffer_with_a_g071_dma_slave() {
        spi_dma_master(c6);
    }

    #[test]
    fn a_g071_dma_master_exchanges_a_buffer_with_an_esp32c6_gdma_slave() {
        spi_dma_slave(c6);
    }

    #[test]
    fn gdma_exchanges_do_not_depend_on_node_order_or_round_size() {
        for env in ["env-spi-esp32c3-dma.yaml", "env-spi-esp32c3-dma-slave.yaml"] {
            assert_order_and_round_independent(env, same, ["esp", "b"], 1500, 16);
        }
        assert_order_and_round_independent("env-spi-esp32c3-dma.yaml", c6, ["esp", "b"], 1500, 16);
    }
}
