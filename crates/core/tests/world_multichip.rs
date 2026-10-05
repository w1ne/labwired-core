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

// Behaviour when a required prebuilt firmware ELF is absent.
//
// By default a missing ELF skips the test, so the workspace `cargo test` gate
// (which has no arm-none-eabi toolchain or STM32CubeL4 pack) and local dev runs
// stay green instead of demanding cross-built artifacts. The dedicated
// `core-iolink-station` CI job builds the ELFs and sets
// `LABWIRED_REQUIRE_IOLINK_ELFS=1`, which turns "missing" into a hard failure —
// so a silently-broken firmware build can't sail through the gate as a no-op
// skip that still reports `ok`.
// Thin wrapper over the shared helper so the skip/fail policy lives in ONE
// place. This decision was previously copy-pasted here and into the sibling
// world test; two copies of a policy is one copy too many, and only one of
// them would have been updated.
fn skip_or_fail_missing_elfs(build_hint: &str) -> bool {
    labwired_core::test_support::skip_or_fail_missing_firmware(
        "iolink",
        "IO-Link station ELF(s)",
        build_hint,
    )
}

#[test]
fn from_manifest_builds_two_cortexm_nodes_and_uart_link() {
    // Two device-FW nodes wired uart2<->uart2. This exercises node construction,
    // ELF load + reset, the UART-link wiring, and lockstep stepping — without
    // needing the master firmware (Task 4).
    if !station_root().join(DEVICE_FW).exists() {
        skip_or_fail_missing_elfs("make -C examples/iolink-dido/firmware");
        return;
    }
    let env = EnvironmentManifest {
        schema_version: "1.0".into(),
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
        rf: None,
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
// drives a real iolinki DEVICE-firmware sensor chip over the shared UART wire and
// reaches OPERATE. Requires the built ELFs (master-fw/master.elf and the iolink-dido
// device ELF); skipped with a clear message if they are missing.
#[test]
fn master_chip_reaches_operate_with_real_sensor_chip() {
    let root = station_root();
    let master_elf = root.join("master-fw/master.elf");
    let device_elf = root.join("../iolink-dido/firmware/iolink_dido.elf");
    if !master_elf.exists() || !device_elf.exists() {
        skip_or_fail_missing_elfs(
            "make -C examples/iolink-station/master-fw && make -C examples/iolink-dido/firmware",
        );
        return;
    }

    let env = EnvironmentManifest::from_file(root.join("env.yaml")).expect("parse env.yaml");
    let mut world = World::from_manifest(env, &root).expect("build station world");

    // Resolve the observability globals straight from the master ELF symbol
    // table rather than hardcoding link addresses — robust to linker/layout
    // changes (e.g. the STM32CubeL4 linker script). 3 == IOLINK_MASTER_STATE_OPERATE.
    let master_bytes = std::fs::read(&master_elf).expect("read master elf");
    let device_bytes = std::fs::read(&device_elf).expect("read device elf");
    let state_addr = labwired_loader::resolve_symbol_in_elf(&master_bytes, "g_master_state")
        .expect("g_master_state symbol in master elf") as u64;
    let pd0_addr = labwired_loader::resolve_symbol_in_elf(&master_bytes, "g_master_pd")
        .expect("g_master_pd symbol in master elf") as u64;
    let device_state_addr = labwired_loader::resolve_symbol_in_elf(&device_bytes, "g_device_state")
        .expect("g_device_state symbol in device elf") as u64;
    const OPERATE: u8 = 3;

    // The sensor publishes its 74HC165 input byte as process data; the
    // sensor/system.yaml presets `inputs: 165` (0xA5). 0xFF is the master's
    // pre-exchange sentinel, so a real PD read must land on 0xA5.
    const EXPECTED_PD: u8 = 0xA5;

    let mut reached_operate = false;
    let mut last_state = 0u8;
    let mut device_state = 0u8;
    let mut pd0 = 0xFFu8;
    for _ in 0..5_000_000u64 {
        world.step_all();
        let master = world.machines.get("master").unwrap();
        last_state = master.read_u8(state_addr).unwrap();
        let device = world.machines.get("sensor1").unwrap();
        device_state = device.read_u8(device_state_addr).unwrap();
        if last_state == OPERATE {
            reached_operate = true;
        }
        pd0 = master.read_u8(pd0_addr).unwrap();
        // Stop once we have proof of a real cyclic PD exchange in OPERATE.
        if reached_operate && pd0 != 0xFF {
            break;
        }
    }

    assert!(
        reached_operate,
        "master chip never reached OPERATE driving the real sensor chip; \
         last_state={last_state:#x} device_state={device_state:#x} pd0={pd0:#x}"
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
        skip_or_fail_missing_elfs(
            "make -C examples/iolink-station/master-fw-4port && make -C examples/iolink-dido/firmware",
        );
        return;
    }

    let env = EnvironmentManifest::from_file(root.join("env4.yaml")).expect("parse env4.yaml");
    let mut world = World::from_manifest(env, &root).expect("build 4-port station");

    // Resolve the per-port observability arrays from the ELF symbol table
    // (robust to linker layout) rather than hardcoding addresses.
    let master_bytes = std::fs::read(&master_elf).expect("read master elf");
    let state = labwired_loader::resolve_symbol_in_elf(&master_bytes, "g_master_state")
        .expect("g_master_state symbol in master elf") as u64; // g_master_state[4]
    let pd = labwired_loader::resolve_symbol_in_elf(&master_bytes, "g_master_pd")
        .expect("g_master_pd symbol in master elf") as u64; // g_master_pd[4]
    const OPERATE: u8 = 3;
    // sensor1..4 input presets (palindrome bytes), bit-order-invariant.
    let expected: [u8; 4] = [0xA5, 0x3C, 0xC3, 0x5A];

    let mut done = false;
    for _ in 0..40_000_000u64 {
        world.step_all();
        let m = world.machines.get("master").unwrap();
        let all = (0..4u64).all(|i| {
            m.read_u8(state + i).unwrap() == OPERATE
                && m.read_u8(pd + i).unwrap() == expected[i as usize]
        });
        if all {
            done = true;
            break;
        }
    }

    let m = world.machines.get("master").unwrap();
    let states: Vec<u8> = (0..4).map(|i| m.read_u8(state + i).unwrap()).collect();
    let pds: Vec<u8> = (0..4).map(|i| m.read_u8(pd + i).unwrap()).collect();
    assert!(
        done,
        "not all 4 ports reached OPERATE with their expected PD; \
         states={states:02x?} pds={pds:02x?} expected={expected:02x?}"
    );
    eprintln!("4-port station: all ports OPERATE; PDs={pds:02x?}");
}

// Folded in from the former tests/world_gpio_net.rs so this suite adds no extra test
// binary (each integration-test binary links the whole core, ~220 MB debug).
mod gpio_net_world {
    // LabWired - Firmware Simulation Platform
    // Copyright (C) 2026 Andrii Shylenko
    //
    // This software is released under the MIT License.
    // See the LICENSE file in the project root for full license information.

    //! GPIO nets between machines: an STM32G0B1 and an ATmega328P joined by an
    //! interrupt line, a ready line and a shared open-drain alert line with a
    //! pull-up (`examples/gpio-net-two-boards`).
    //!
    //! The firmware counts edges (EXTI on the STM32, INT1 and PCINT2 on the AVR) and
    //! reports over UART. Every count is a hand-derived number from the firmware's
    //! loops, not a measurement: 10 irq pulses, 7 ready pulses, 5 alert pulses the
    //! STM32 pulls and 3 the AVR pulls.

    use labwired_config::EnvironmentManifest;
    use labwired_core::network::gpio_net::{GPIO_NET_CONTENTION, GPIO_NET_FLOATING};
    use labwired_core::world::World;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn example() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/gpio-net-two-boards")
    }

    type Sinks = (Arc<Mutex<Vec<u8>>>, Arc<Mutex<Vec<u8>>>);

    fn build(env_file: &str, rewrite: impl Fn(String) -> String) -> (World, Sinks) {
        let yaml = rewrite(std::fs::read_to_string(example().join(env_file)).unwrap());
        let manifest: EnvironmentManifest = serde_yaml::from_str(&yaml).unwrap();
        let mut world = World::from_manifest(manifest, &example()).expect("world");
        let stm = Arc::new(Mutex::new(Vec::new()));
        let avr = Arc::new(Mutex::new(Vec::new()));
        for (id, sink) in [("stm", &stm), ("avr", &avr)] {
            // Renamed nodes keep their role in the id's tail.
            let key = world
                .machines
                .keys()
                .find(|k| k.ends_with(id))
                .cloned()
                .unwrap();
            world
                .machines
                .get_mut(&key)
                .unwrap()
                .attach_uart_tx_sink(sink.clone(), false)
                .unwrap();
        }
        (world, (stm, avr))
    }

    fn run_ms(world: &mut World, ms: u64) {
        let end = ms * 1_000_000_000;
        let mut calls = 0u64;
        while world.round_now_ps().unwrap() < end {
            for (id, r) in world.step_all() {
                r.unwrap_or_else(|e| panic!("node {id}: {e:?}"));
            }
            calls += 1;
            assert!(calls < 50_000_000, "runaway");
        }
    }

    fn text(sink: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8_lossy(&sink.lock().unwrap()).into_owned()
    }

    fn stm_result(world: &World, id: &str) -> Vec<u32> {
        let b = world.machines[id].read_memory(0x2000_0100, 20).unwrap();
        b.chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    const STM_LINE: &str = "STM irq r=10 f=10 alert r=3 f=3\n";
    const AVR_LINE: &str = "AVR ready=7 alert f=5 r=5\n";

    /// Everything one run can show that must not depend on how it was scheduled.
    #[derive(Debug, PartialEq)]
    struct Fingerprint {
        stm_uart: String,
        avr_uart: String,
        stm_ram: Vec<u32>,
        applied: Vec<(String, u8, u64, u64, bool)>,
        nets: Vec<(String, u64, bool, u64)>,
    }

    fn fingerprint(world: &World, stm_id: &str, sinks: &Sinks) -> Fingerprint {
        Fingerprint {
            stm_uart: text(&sinks.0),
            avr_uart: text(&sinks.1),
            stm_ram: stm_result(world, stm_id),
            applied: world
                .gpio_net_applied()
                .iter()
                .map(|a| (a.node.clone(), a.pin, a.cycle, a.due_ps, a.level))
                .collect(),
            nets: world
                .gpio_net_reports()
                .iter()
                .map(|n| (n.name.clone(), n.edges, n.level, n.contention_events))
                .collect(),
        }
    }

    /// The browser lab runs the same firmware 20x slower so a person can watch
    /// the pulses (`env-demo.yaml`, built with `-DTIME_SCALE=20`). Slower must
    /// not change a single count.
    #[test]
    fn the_demo_timing_counts_the_same_as_the_fast_one() {
        let (mut world, sinks) = build("env-demo.yaml", |s| s);
        run_ms(&mut world, 1000);
        assert_eq!(text(&sinks.0), STM_LINE, "STM32 report");
        assert_eq!(text(&sinks.1), AVR_LINE, "AVR report");
        assert_eq!(stm_result(&world, "stm"), vec![10, 10, 3, 3, 1]);
        let edges: Vec<(String, u64)> = world
            .gpio_net_reports()
            .iter()
            .map(|n| (n.name.clone(), n.edges))
            .collect();
        assert_eq!(
            edges,
            vec![
                ("irq".into(), 20),
                ("ready".into(), 14),
                ("alert".into(), 16)
            ]
        );
    }

    #[test]
    fn two_boards_count_each_others_edges_exactly() {
        let (mut world, sinks) = build("env.yaml", |s| s);
        run_ms(&mut world, 30);
        assert_eq!(text(&sinks.0), STM_LINE, "STM32 report");
        assert_eq!(text(&sinks.1), AVR_LINE, "AVR report");
        assert_eq!(stm_result(&world, "stm"), vec![10, 10, 3, 3, 1]);

        let reports = world.gpio_net_reports();
        let by = |n: &str| reports.iter().find(|r| r.name == n).unwrap();
        // irq: 10 pulses = 20 wire edges; ready: 7 pulses = 14; alert: 5 + 3
        // pulses = 16 (open-drain wired-AND, pull-up).
        assert_eq!(by("irq").edges, 20);
        assert_eq!(by("ready").edges, 14);
        assert_eq!(by("alert").edges, 16);
        for r in &reports {
            assert_eq!(r.contention_events, 0, "{}", r.name);
            // Nothing drives a pulled net, and the ready line has no driver
            // until the STM32 firmware configures it: only that is flagged.
            assert!(r.diagnostics.iter().all(|d| d.code != GPIO_NET_CONTENTION));
        }
        assert_eq!(
            by("alert").floating_events,
            0,
            "the pull-up holds alert high"
        );
    }

    /// Same run, scheduled differently: node ids that sort the other way round
    /// (so the world steps the AVR first), a different manifest order for nets
    /// and nodes, and rounds of several lengths. Every observable is identical.
    #[test]
    fn results_do_not_depend_on_node_order_or_round_size() {
        let baseline = {
            let (mut w, s) = build("env.yaml", |s| s);
            run_ms(&mut w, 30);
            fingerprint(&w, "stm", &s)
        };
        assert_eq!(baseline.stm_uart, STM_LINE);

        // Node order: the world steps nodes in id order. `avr` < `stm`, so the
        // baseline steps the AVR first; `a_stm` < `z_avr` steps the STM32 first.
        let renamed = |s: String| {
            s.replace("id: stm", "id: a_stm")
                .replace("id: avr", "id: z_avr")
                .replace("node: stm", "node: a_stm")
                .replace("node: avr", "node: z_avr")
                .replace("[avr, stm]", "[a_stm, z_avr]")
        };
        let (mut w, s) = build("env.yaml", renamed);
        run_ms(&mut w, 30);
        let f = fingerprint(&w, "a_stm", &s);
        assert_eq!(f.stm_uart, baseline.stm_uart);
        assert_eq!(f.avr_uart, baseline.avr_uart);
        assert_eq!(f.stm_ram, baseline.stm_ram);
        // The applied log names nodes; compare with the names mapped back.
        let norm = |v: &[(String, u8, u64, u64, bool)]| {
            let mut v: Vec<_> = v
                .iter()
                .map(|(n, p, c, d, l)| {
                    (
                        n.trim_start_matches("a_")
                            .trim_start_matches("z_")
                            .to_string(),
                        *p,
                        *c,
                        *d,
                        *l,
                    )
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(norm(&f.applied), norm(&baseline.applied), "node order");
        assert_eq!(f.nets, baseline.nets);

        // Round size: one tenth, a prime fraction, and the full latency.
        for round_ps in [10_000u64, 33_333, 70_001, 100_000] {
            let (mut w, s) = build("env.yaml", |s| s);
            w.set_gpio_round_ps(round_ps).unwrap();
            run_ms(&mut w, 30);
            let f = fingerprint(&w, "stm", &s);
            assert_eq!(f, baseline, "round {round_ps} ps");
        }

        // The same net with a different latency is a different run (the delay is
        // real), but it still counts the same edges.
        let (mut w, s) = build("env.yaml", |s| {
            s.replace("latency_ns: 100", "latency_ns: 1000")
        });
        run_ms(&mut w, 30);
        assert_eq!(text(&s.0), STM_LINE);
        assert_eq!(text(&s.1), AVR_LINE);
    }

    /// The ATmega328P pads on the nets are captured by push, not by the
    /// per-cycle poll, so neither node is clamped to one instruction per
    /// batch. The AVR counts `ready` with INT1 and `alert` with PCINT2 and
    /// sleeps between edges (src/avr.c): the right counts prove the edges
    /// arrived as interrupts, since nothing polls PIND any more.
    #[test]
    fn avr_net_pads_are_push_captured_and_counted_by_interrupts() {
        let (mut world, sinks) = build("env.yaml", |s| s);
        for (id, m) in &world.machines {
            assert!(
                !m.logic_poll_active(),
                "node {id} fell back to poll capture"
            );
        }
        run_ms(&mut world, 30);
        assert_eq!(text(&sinks.1), AVR_LINE, "AVR report");
        assert_eq!(text(&sinks.0), STM_LINE, "STM32 report");
    }

    /// With idle fast-forward on, the sleeping AVR skips its waits and every
    /// observable stays identical to the run that steps each idle clock.
    #[cfg(feature = "event-scheduler")]
    #[test]
    fn idle_fast_forward_skips_the_avr_sleep_and_changes_nothing() {
        let baseline = {
            let (mut w, s) = build("env.yaml", |s| s);
            run_ms(&mut w, 30);
            assert_eq!(w.machines["avr"].idle_fast_forward_cycles(), 0);
            fingerprint(&w, "stm", &s)
        };
        let (mut w, s) = build("env.yaml", |s| s);
        for m in w.machines.values_mut() {
            m.set_idle_fast_forward(true);
        }
        run_ms(&mut w, 30);
        let skipped = w.machines["avr"].idle_fast_forward_cycles();
        assert_eq!(fingerprint(&w, "stm", &s), baseline);
        // The AVR sleeps from the end of its irq pulses until the last alert
        // edge, about 0.9 ms (14 400 cycles at 16 MHz); a round of 100 ns is
        // under two cycles, so most of each sleeping round is skipped.
        assert!(
            skipped > 5_000,
            "the sleeping AVR must fast-forward, skipped {skipped} cycles"
        );
    }

    #[test]
    fn both_boards_driving_one_push_pull_wire_reports_contention() {
        let (mut world, _s) = build("env-contention.yaml", |s| s);
        run_ms(&mut world, 1);
        let reports = world.gpio_net_reports();
        assert_eq!(reports.len(), 1);
        let net = &reports[0];
        assert_eq!(net.name, "fight");
        assert_eq!(net.contention_events, 1, "{net:#?}");
        let d = net
            .diagnostics
            .iter()
            .find(|d| d.code == GPIO_NET_CONTENTION)
            .expect("GPIO_NET_CONTENTION");
        // The STM32 drives high for about 150 loop iterations (a few tens of us)
        // after boot; the AVR has held the wire low since its first instructions.
        assert!(
            d.t_ps > 1_000_000 && d.t_ps < 200_000_000,
            "begins at {} ps",
            d.t_ps
        );
        let end = d.end_ps.expect("the STM32 releases the wire");
        assert!(end > d.t_ps);
        let drives: Vec<_> = d.members.iter().map(|m| (m.node.as_str(), m.pin)).collect();
        assert_eq!(drives, vec![("avr", 2), ("stm", 1)]);
        // The low side wins: the wire never rises, so nothing was delivered.
        assert!(!net.level);
        assert_eq!(net.edges, 0);
        assert!(world.gpio_net_applied().iter().all(|a| !a.level));
    }

    #[test]
    fn a_net_nobody_drives_or_pulls_floats_and_is_flagged() {
        let (mut world, _s) = build("env.yaml", |s| s);
        run_ms(&mut world, 0);
        // The ready wire has a pull-down, so it is not floating; remove it.
        let (mut world2, _s2) = build("env.yaml", |s| {
            s.replacen(
                "      name: ready\n      pull: down\n",
                "      name: ready\n",
                1,
            )
        });
        run_ms(&mut world2, 0);
        let ready = world2
            .gpio_net_reports()
            .into_iter()
            .find(|r| r.name == "ready")
            .unwrap();
        assert_eq!(ready.floating_events, 1);
        assert!(ready
            .diagnostics
            .iter()
            .any(|d| d.code == GPIO_NET_FLOATING));
        assert!(!ready.level, "a floating wire reads 0");
        let ready = world
            .gpio_net_reports()
            .into_iter()
            .find(|r| r.name == "ready")
            .unwrap();
        assert_eq!(ready.floating_events, 0);
    }

    /// The same STM32 firmware with another chip in place of the ATmega328P
    /// (`env-rp2040.yaml`, `env-esp32c6.yaml`). The peer counts the STM32's
    /// edges with its GPIO interrupt and leaves `[ready rising, alert
    /// falling, alert rising, interrupts taken, done]` at `result`.
    fn build_peer(
        env_file: &str,
        rewrite: impl Fn(String) -> String,
    ) -> (World, Arc<Mutex<Vec<u8>>>) {
        let yaml = rewrite(std::fs::read_to_string(example().join(env_file)).unwrap());
        let manifest: EnvironmentManifest = serde_yaml::from_str(&yaml).unwrap();
        let mut world = World::from_manifest(manifest, &example()).expect("world");
        let stm = Arc::new(Mutex::new(Vec::new()));
        let key = world
            .machines
            .keys()
            .find(|k| k.ends_with("stm"))
            .cloned()
            .unwrap();
        world
            .machines
            .get_mut(&key)
            .unwrap()
            .attach_uart_tx_sink(stm.clone(), false)
            .unwrap();
        (world, stm)
    }

    fn peer_result(world: &World, id: &str, at: u32) -> Vec<u32> {
        let b = world.machines[id].read_memory(at, 20).unwrap();
        b.chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    /// One peer chip against the unchanged STM32 firmware: both sides count
    /// every edge, the peer with GPIO interrupts, and neither node order nor
    /// round length changes a number.
    fn peer_counts_the_stm32_edges(env_file: &str, peer: &str, result: u32) {
        let (mut world, stm) = build_peer(env_file, |s| s);
        run_ms(&mut world, 30);
        assert_eq!(text(&stm), STM_LINE, "STM32 report");
        assert_eq!(stm_result(&world, "stm"), vec![10, 10, 3, 3, 1]);
        let r = peer_result(&world, peer, result);
        assert_eq!(&r[..3], &[7, 5, 5], "{peer} interrupt counts {r:?}");
        assert!(r[3] >= 17, "at least one interrupt per edge {r:?}");
        assert_eq!(r[4], 1, "{peer} finished {r:?}");
        let reports = world.gpio_net_reports();
        let by = |n: &str| reports.iter().find(|r| r.name == n).unwrap();
        assert_eq!(by("irq").edges, 20);
        assert_eq!(by("ready").edges, 14);
        assert_eq!(by("alert").edges, 16);
        for r in &reports {
            assert_eq!(r.contention_events, 0, "{}", r.name);
            assert_eq!(r.floating_events, 0, "{}", r.name);
        }

        let fp = |w: &World, stm: &str, p: &str, s: &Arc<Mutex<Vec<u8>>>| {
            (
                text(s),
                stm_result(w, stm),
                peer_result(w, p, result),
                w.gpio_net_reports()
                    .iter()
                    .map(|n| (n.name.clone(), n.edges, n.level))
                    .collect::<Vec<_>>(),
            )
        };
        let baseline = fp(&world, "stm", peer, &stm);
        let renamed = format!("z_{peer}");
        let (mut w, s) = build_peer(env_file, |s| {
            s.replace("id: stm", "id: a_stm")
                .replace(&format!("id: {peer}"), &format!("id: {renamed}"))
                .replace("node: stm", "node: a_stm")
                .replace(&format!("node: {peer}"), &format!("node: {renamed}"))
                .replace(&format!("[{peer}, stm]"), &format!("[a_stm, {renamed}]"))
        });
        run_ms(&mut w, 30);
        assert_eq!(fp(&w, "a_stm", &renamed, &s), baseline, "node order");
        let (mut w, s) = build_peer(env_file, |s| s);
        w.set_gpio_round_ps(33_333).unwrap();
        run_ms(&mut w, 30);
        assert_eq!(fp(&w, "stm", peer, &s), baseline, "round length");
    }

    /// RP2040 SIO pads on the nets; IO_BANK0 `EDGE_HIGH` / `EDGE_LOW`
    /// interrupts (IO_IRQ_BANK0, NVIC 13) count the STM32's edges, and the
    /// alert pad is open drain by `GPIO_OE`.
    #[test]
    fn an_rp2040_counts_the_stm32_edges_with_gpio_interrupts() {
        peer_counts_the_stm32_edges("env-rp2040.yaml", "rp", 0x2000_0100);
    }

    /// ESP32-C6 GPIO pads on the nets; `GPIO_PINn.INT_TYPE` interrupts through
    /// the interrupt matrix (source 30 -> CPU line 9) count the STM32's edges,
    /// and the alert pad is open drain by `GPIO_PIN6.PAD_DRIVER`.
    #[test]
    fn an_esp32c6_counts_the_stm32_edges_with_gpio_interrupts() {
        peer_counts_the_stm32_edges("env-esp32c6.yaml", "c6", 0x4080_0100);
    }

    fn build_err(env_file: &str, rewrite: impl Fn(String) -> String) -> String {
        let yaml = rewrite(std::fs::read_to_string(example().join(env_file)).unwrap());
        match serde_yaml::from_str::<EnvironmentManifest>(&yaml)
            .map_err(anyhow::Error::from)
            .and_then(|m| World::from_manifest(m, &example()))
        {
            Ok(_) => panic!("expected the world to be refused"),
            Err(e) => format!("{e:#}"),
        }
    }

    #[test]
    fn zero_latency_and_sub_cycle_latency_are_refused() {
        let e = build_err("env.yaml", |s| {
            s.replace("latency_ns: 100", "latency_ns: 0")
        });
        assert!(e.contains("zero-delay"), "{e}");
        // One 16 MHz cycle is 62.5 ns.
        let e = build_err("env.yaml", |s| {
            s.replace("latency_ns: 100", "latency_ns: 40")
        });
        assert!(e.contains("below one cycle"), "{e}");
    }

    #[test]
    fn a_pad_on_two_nets_and_an_unknown_pad_are_refused() {
        let e = build_err("env.yaml", |s| {
            s.replace(
                "{ node: avr, peripheral: portd, pin: 3 }",
                "{ node: avr, peripheral: portd, pin: 2 }",
            )
        });
        assert!(e.contains("on two nets"), "{e}");
        let e = build_err("env.yaml", |s| {
            s.replace("peripheral: portd, pin: 3", "peripheral: portx, pin: 3")
        });
        assert!(e.contains("portx"), "{e}");
        let e = build_err("env.yaml", |s| s.replace("pin: 3 }", "pin: 9 }"));
        assert!(
            e.to_lowercase().contains("pin") || e.contains("net support"),
            "{e}"
        );
    }
}

// STM32F1 / F4 EXTI on a net, and a chip's internal pull-up as the only pull
// on a wire (`examples/gpio-net-f1-f4`).
mod gpio_net_f1_f4 {
    //! An STM32F103, an STM32F401 and an ATmega328P. The AVR puts 10 pulses
    //! on `irq`; the F103 counts them with EXTI0 (AFIO_EXTICR1 = port B) and
    //! the F401 with EXTI1 (SYSCFG_EXTICR1 = port C), rising and falling
    //! separately. `alert` and `wake` have no `pull`: the F401's PUPDR
    //! pull-up holds `alert` high while the AVR pulls it low 3 times (the F401
    //! counts them on EXTI8), and the AVR's own pull-up (PORTD5 with DDRD5
    //! clear) holds `wake` high while the F103 pulls it low 4 times (the AVR
    //! polls them). Every count is the firmware's loop count.

    use labwired_config::EnvironmentManifest;
    use labwired_core::network::gpio_net::{
        Own, GPIO_NET_CONTENTION, GPIO_NET_FLOATING, GPIO_NET_PULL_CONFLICT,
    };
    use labwired_core::world::World;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn example() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/gpio-net-f1-f4")
    }

    fn build(rewrite: impl Fn(String) -> String) -> (World, Arc<Mutex<Vec<u8>>>) {
        let yaml = rewrite(std::fs::read_to_string(example().join("env.yaml")).unwrap());
        let manifest: EnvironmentManifest = serde_yaml::from_str(&yaml).unwrap();
        let mut world = World::from_manifest(manifest, &example()).expect("world");
        let avr = Arc::new(Mutex::new(Vec::new()));
        world
            .machines
            .get_mut("avr")
            .unwrap()
            .attach_uart_tx_sink(avr.clone(), false)
            .unwrap();
        (world, avr)
    }

    fn run_ms(world: &mut World, ms: u64) {
        let end = ms * 1_000_000_000;
        let mut calls = 0u64;
        while world.round_now_ps().unwrap() < end {
            for (id, r) in world.step_all() {
                r.unwrap_or_else(|e| panic!("node {id}: {e:?}"));
            }
            calls += 1;
            assert!(calls < 50_000_000, "runaway");
        }
    }

    fn ram(world: &World, id: &str, words: usize) -> Vec<u32> {
        let b = world.machines[id]
            .read_memory(0x2000_0100, words * 4)
            .unwrap();
        b.chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    #[test]
    fn f1_and_f4_exti_count_a_peers_edges_and_an_internal_pull_up_holds_the_wire() {
        let (mut world, avr) = build(|s| s);
        run_ms(&mut world, 20);

        // F103: 10 rising + 10 falling irq edges through AFIO; wake done.
        assert_eq!(ram(&world, "f1", 3), vec![10, 10, 1], "F103 EXTI0");
        // F401: 10 + 10 irq edges through SYSCFG, 3 + 3 alert edges.
        assert_eq!(ram(&world, "f4", 4), vec![10, 10, 3, 3], "F401 EXTI1/EXTI8");
        // The AVR saw the F103's 4 pulses on a wire only its pull-up lifts.
        assert_eq!(
            String::from_utf8_lossy(&avr.lock().unwrap()),
            "AVR wake f=4 r=4\n"
        );

        let reports = world.gpio_net_reports();
        let by = |n: &str| reports.iter().find(|r| r.name == n).unwrap();
        assert_eq!(by("irq").edges, 20);
        // alert and wake float until the firmware turns the pull-up on, rise
        // once when it does, then carry 3 and 4 pulses.
        assert_eq!(by("alert").edges, 1 + 6);
        assert_eq!(by("wake").edges, 1 + 8);
        for n in ["alert", "wake"] {
            let r = by(n);
            assert!(r.level, "{n} rests high on the internal pull-up");
            assert_eq!(
                r.floating_events, 1,
                "{n}: floating only before the pull-up"
            );
            let d = r
                .diagnostics
                .iter()
                .find(|d| d.code == GPIO_NET_FLOATING)
                .unwrap();
            assert_eq!(d.t_ps, 0);
            assert!(d.end_ps.is_some(), "{n}: the pull-up ends the float");
        }
        for r in &reports {
            assert_eq!(r.contention_events, 0, "{}", r.name);
            assert_eq!(r.pull_conflict_events, 0, "{}", r.name);
            assert!(r.diagnostics.iter().all(|d| d.code != GPIO_NET_CONTENTION));
        }
        let drive = |net: &str, node: &str| {
            by(net)
                .members
                .iter()
                .find(|m| m.node == node)
                .unwrap()
                .drive
        };
        assert_eq!(drive("alert", "f4"), Own::PullUp);
        assert_eq!(drive("alert", "avr"), Own::Z);
        assert_eq!(drive("wake", "avr"), Own::PullUp);
        assert_eq!(drive("wake", "f1"), Own::Z, "released open-drain");
    }

    #[test]
    fn an_internal_pull_up_against_the_nets_pull_down_is_a_pull_conflict() {
        let (mut world, _avr) =
            build(|s| s.replace("      name: wake\n", "      name: wake\n      pull: down\n"));
        run_ms(&mut world, 2);
        let wake = world
            .gpio_net_reports()
            .into_iter()
            .find(|r| r.name == "wake")
            .unwrap();
        assert_eq!(wake.pull, "down");
        assert_eq!(wake.pull_conflict_events, 1, "{wake:#?}");
        let d = wake
            .diagnostics
            .iter()
            .find(|d| d.code == GPIO_NET_PULL_CONFLICT)
            .unwrap();
        assert!(d.end_ps.is_none(), "the divider lasts");
        // The board resistor decides: the wire reads low and never rose.
        assert!(!wake.level);
        assert_eq!(wake.edges, 0);
        assert_eq!(wake.floating_events, 0);
    }
}
