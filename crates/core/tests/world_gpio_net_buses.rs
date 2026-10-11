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
    let mut level = idle;
    let mut edges = 0;
    for &b in bytes {
        for i in (0..8).rev() {
            let bit = (b >> i) & 1 != 0;
            if bit != level {
                edges += 1;
                level = bit;
            }
        }
    }
    edges
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
