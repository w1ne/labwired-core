// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A timed UART network of real firmware (Renode issue #948's question):
//! "when this link slows down and one node restarts, does the network recover
//! without overflowing its buffers?"
//!
//! The nodes are STM32F401 (NUCLEO-F401RE) running `uart-chain.c`, an
//! interrupt-driven relay: USART1 RXNE interrupt into a ring buffer, a main
//! loop that parses 5-byte messages and forwards them with the hop count
//! incremented, USART2 TXE interrupt draining a TX ring buffer. Node 0 is the
//! source (one message per millisecond from SysTick).
//!
//! Every golden number is computed from first principles, not measured:
//!
//! * 115200 baud from 84 MHz: BRR = 0x2D9 = 729 cycles per bit.
//! * 8N1: a character is 10 bits on the wire; the receiver sets RXNE in the
//!   middle of the stop bit, 9.5 bits after the start edge.
//! * A 5-byte message leaves back to back (the TXE interrupt refills the data
//!   register while the shifter works), so its last character starts 4 frames
//!   after the first and completes 4 × 10 + 9.5 = 49.5 bit times after the
//!   first start bit. That is one hop, `HOP`.
//! * Across `k` links: `k × HOP + (k − 1) × p`, where `p` is the relay's own
//!   store-and-forward time (interrupt entry, parse, queue, TX interrupt).
//!   `p` is the firmware's, so the test measures it and bounds it.

use labwired_config::{ChipDescriptor, EnvironmentManifest, SystemManifest};
use labwired_core::network::timed_uart::{NetEventKind, NetReport};
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{ResolvedWorldNode, World};
use std::path::PathBuf;

const HZ: u64 = 84_000_000;
const BRR: u64 = 729;
const MSG_LEN: u64 = 5;
const PERIOD_PS: u64 = 1_000_000_000; // 1 ms
const RESULT: u32 = 0x2000_0100;
const BOOTS: u32 = 0x2000_00F0;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One bit at 115200 baud from 84 MHz, ps.
fn bit_ps() -> u64 {
    labwired_core::network::timed_uart::cycles_to_ps(BRR, HZ)
}

/// One cycle, ps (the model's resolution).
fn cycle_ps() -> u64 {
    labwired_core::network::timed_uart::cycles_to_ps(1, HZ) + 1
}

/// One hop of one message: (MSG_LEN − 1) whole frames + 9.5 bits, ps.
fn hop_ps() -> u64 {
    (MSG_LEN - 1) * 10 * bit_ps() + bit_ps() * 19 / 2
}

fn elf(name: &str) -> Vec<u8> {
    std::fs::read(
        root()
            .join("crates/core/tests/fixtures/uart-chain")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A world of `firmwares.len()` NUCLEO-F401RE nodes `n0, n1, …` joined by a
/// `uart_network` with the given extra config lines.
fn world(firmwares: &[&str], config: &str) -> World {
    world_with(firmwares, config, "chain")
}

fn world_with(firmwares: &[&str], config: &str, topology: &str) -> World {
    let chip = ChipDescriptor::from_file(root().join("configs/chips/stm32f401.yaml")).unwrap();
    let system =
        SystemManifest::from_file(root().join("configs/systems/nucleo-f401re.yaml")).unwrap();
    let ids: Vec<String> = (0..firmwares.len()).map(|i| format!("n{i}")).collect();
    let nodes_yaml: String = ids
        .iter()
        .map(|id| format!("  - {{ id: {id}, system: s.yaml, firmware: f.elf }}\n"))
        .collect();
    let markers: String = ids
        .iter()
        .map(|id| format!("      - {{ node: {id}, peripheral: gpioa, pin: 5, name: app }}\n"))
        .collect();
    let yaml = format!(
        r#"schema_version: "1.0"
name: uart-chain
nodes:
{nodes_yaml}interconnects:
  - type: uart_network
    nodes: [{list}]
    config:
      topology: {topology}
      messages: {{ sync: 0xA5, length: 5, id_offset: 1, id_bytes: 2, hop_offset: 3, checksum_xor: true }}
      markers:
{markers}{config}"#,
        list = ids.join(", "),
    );
    let manifest: EnvironmentManifest = serde_yaml::from_str(&yaml).unwrap();
    let resolved = ids
        .iter()
        .zip(firmwares)
        .map(|(id, fw)| ResolvedWorldNode {
            id: id.clone(),
            system: system.clone(),
            chip: chip.clone(),
            firmware: NodeFirmware::from_bytes(elf(fw)),
            blobs: Default::default(),
        })
        .collect();
    World::from_resolved(manifest, resolved).expect("world")
}

fn run_to(world: &mut World, t_ps: u64) {
    let mut rounds = 0u64;
    while world.uart_network_now_ps().unwrap() < t_ps {
        for (id, r) in world.step_all() {
            r.unwrap_or_else(|e| panic!("node {id}: {e:?}"));
        }
        rounds += 1;
        assert!(rounds < 10_000_000, "runaway");
    }
}

fn result_words(world: &World, node: &str) -> Vec<u32> {
    let bytes = world.machines[node].read_memory(RESULT, 48).unwrap();
    bytes
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn boots(world: &World, node: &str) -> u32 {
    let b = world.machines[node].read_memory(BOOTS, 4).unwrap();
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn report(world: &World) -> NetReport {
    world.uart_network_report(0).unwrap()
}

/// Latency of message `id` to `node`, ps.
fn latency(r: &NetReport, id: u32, node: &str) -> Option<u64> {
    r.messages
        .iter()
        .find(|m| m.id == id)?
        .deliveries
        .iter()
        .find(|d| d.node == node)?
        .latency_ps
}

const SOURCE: &str = "uart-chain-source.elf";
const RELAY: &str = "uart-chain-relay.elf";

/// Message `id` leaves the source `(id + 1)` ms after boot (SysTick period)
/// plus the source's own few-µs queueing time.
fn origin(r: &NetReport, id: u32) -> Option<u64> {
    r.messages.iter().find(|m| m.id == id)?.origin_ps
}

fn ids_delivered_to(r: &NetReport, node: &str) -> Vec<u32> {
    let mut v: Vec<u32> = r
        .messages
        .iter()
        .filter(|m| m.deliveries.iter().any(|d| d.node == node))
        .map(|m| m.id)
        .collect();
    v.sort();
    v
}

fn n2_checksum_failures(w: &World) -> u32 {
    result_words(w, "n2")[3]
}

fn ps_to_us(ps: u64) -> f64 {
    ps as f64 / 1e6
}

/// The golden checks of the 3-node baseline, as a `Result` so the negative
/// control can show they FAIL without the wire model.
fn check_baseline_golden(r: &NetReport) -> Result<(u64, u64), String> {
    let hop = hop_ps();
    let tol = 2 * cycle_ps();
    let mut p_min = u64::MAX;
    let mut p_max = 0;
    let mut checked = 0;
    for m in &r.messages {
        let (Some(l1), Some(l2)) = (latency(r, m.id, "n1"), latency(r, m.id, "n2")) else {
            continue;
        };
        if l1.abs_diff(hop) > tol {
            return Err(format!(
                "msg {}: latency to n1 {} ps, predicted 1 hop = {} ps",
                m.id, l1, hop
            ));
        }
        let Some(p) = l2.checked_sub(2 * hop) else {
            return Err(format!(
                "msg {}: latency to n2 {} ps is below 2 hops = {} ps",
                m.id,
                l2,
                2 * hop
            ));
        };
        if p == 0 || p > 10_000_000 {
            return Err(format!(
                "msg {}: relay store-and-forward time {} ps is outside (0, 10 µs]",
                m.id, p
            ));
        }
        p_min = p_min.min(p);
        p_max = p_max.max(p);
        checked += 1;
    }
    if checked < 8 {
        return Err(format!("only {checked} messages crossed both hops"));
    }
    Ok((p_min, p_max))
}

/// Baseline: every message arrives, one hop costs exactly 49.5 bit times,
/// and the unified timeline orders transmit → delivery → interrupt → app
/// marker → forward.
#[test]
fn baseline_chain_delivers_every_message_at_the_predicted_latency() {
    let mut w = world(&[SOURCE, RELAY, RELAY], "");
    if std::env::var_os("LABWIRED_UART_NET_NEGATIVE_CONTROL").is_some() {
        // Manual negative control: the pre-timed behaviour (instant delivery).
        w.uart_network().unwrap().debug_instant_delivery(true);
    }
    run_to(&mut w, 12 * PERIOD_PS);
    let r = report(&w);

    let (p_min, p_max) = check_baseline_golden(&r).unwrap_or_else(|e| panic!("{e}"));
    let first = &r.messages[0];
    let l1 = latency(&r, 0, "n1").unwrap();
    let l2 = latency(&r, 0, "n2").unwrap();
    eprintln!(
        "GOLDEN hop = 49.5 bits x {} ps = {} ps ({:.3} us)",
        bit_ps(),
        hop_ps(),
        ps_to_us(hop_ps())
    );
    eprintln!(
        "MEASURED msg 0: origin {:.3} us, n1 {:.3} us (1 hop, predicted {:.3}), n2 {:.3} us (2 hops + p, predicted {:.3} + p)",
        ps_to_us(first.origin_ps.unwrap()),
        ps_to_us(l1),
        ps_to_us(hop_ps()),
        ps_to_us(l2),
        ps_to_us(2 * hop_ps())
    );
    eprintln!(
        "MEASURED relay store-and-forward p: {:.3}..{:.3} us",
        ps_to_us(p_min),
        ps_to_us(p_max)
    );

    // Every message the source sent reached the end, in order.
    let sent = r
        .messages
        .iter()
        .filter(|m| m.origin_node.as_deref() == Some("n0"))
        .count();
    let at_n2 = ids_delivered_to(&r, "n2");
    assert!(sent >= 11, "source sent {sent}");
    // The last one or two may still be on the wire at the end of the run.
    assert!(at_n2.len() + 2 >= sent, "{} of {sent} arrived", at_n2.len());
    assert_eq!(at_n2, (0..at_n2.len() as u32).collect::<Vec<_>>());
    let n2 = result_words(&w, "n2");
    assert_eq!(n2[0] as usize, at_n2.len(), "firmware count agrees");
    assert_eq!(n2[2], 1, "n2 sees hop count 1");
    assert_eq!(
        (n2[3], n2[4], n2[5], n2[10]),
        (0, 0, 0, 0),
        "no checksum/ORE/FE/gap"
    );
    assert!(n2[6] <= 1, "RX ring high-water {}: the ISR keeps up", n2[6]);

    // Per-character latency is 9.5 bit times exactly, on both links.
    for link in &r.links {
        let s = &link.directions[0].stats;
        assert_eq!((s.overruns, s.framing_errors, s.late), (0, 0, 0), "{s:?}");
        let per_char = bit_ps() * 19 / 2;
        assert!(s.latency_min_ps.unwrap().abs_diff(per_char) <= cycle_ps());
        assert!(s.latency_max_ps.unwrap().abs_diff(per_char) <= cycle_ps());
        let tput = link.directions[0].throughput_chars_per_s.unwrap();
        // 5 characters per 1 ms message.
        assert!((4_500.0..5_600.0).contains(&tput), "throughput {tput}");
        eprintln!(
            "LINK {} {}->{}: {} chars, {:.0} chars/s, utilization {:.3}, max in flight {}",
            link.id,
            s.from_node,
            s.to_node,
            s.chars_delivered,
            tput,
            link.directions[0].utilization.unwrap(),
            s.max_in_flight
        );
    }

    // One timeline: for message 1 at the relay, the last character's delivery
    // precedes the RX interrupt, which precedes the application marker, which
    // precedes the forwarded first character.
    let t_deliver = r
        .messages
        .iter()
        .find(|m| m.id == 1)
        .unwrap()
        .deliveries
        .iter()
        .find(|d| d.node == "n1")
        .unwrap()
        .t_ps;
    let after = |kind: NetEventKind, node: &str, t: u64| {
        r.events
            .iter()
            .find(|e| e.kind == kind && e.node == node && e.t_ps >= t)
            .map(|e| e.t_ps)
            .unwrap_or_else(|| panic!("no {kind:?} at {node} after {t}"))
    };
    let t_irq = after(NetEventKind::RxIrq, "n1", t_deliver);
    let t_marker = after(NetEventKind::Marker, "n1", t_irq);
    let t_fwd = after(NetEventKind::TxStart, "n1", t_marker);
    eprintln!(
        "TIMELINE msg 1 at n1: deliver {:.3} us -> rx_irq +{:.3} us -> app marker +{:.3} us -> forward tx_start +{:.3} us",
        ps_to_us(t_deliver),
        ps_to_us(t_irq - t_deliver),
        ps_to_us(t_marker - t_deliver),
        ps_to_us(t_fwd - t_deliver)
    );
    assert!(t_fwd - t_deliver <= p_max + cycle_ps());
}

/// NEGATIVE CONTROL: with the wire model switched to instant delivery (what
/// the untimed cross-link does), the same golden check fails.
#[test]
fn negative_control_instant_delivery_fails_the_latency_golden() {
    let mut w = world(&[SOURCE, RELAY, RELAY], "");
    w.uart_network().unwrap().debug_instant_delivery(true);
    run_to(&mut w, 12 * PERIOD_PS);
    match check_baseline_golden(&report(&w)) {
        Ok((p_min, p_max)) => panic!(
            "instant delivery passed the latency golden (p {p_min}..{p_max}): the check is vacuous"
        ),
        Err(err) => eprintln!("NEGATIVE CONTROL (instant delivery): {err}"),
    }
}

/// Slowing link 1 (n1 → n2) by 500 µs raises the end-to-end latency of every
/// later message by exactly 500 µs, and leaves link 0 alone.
#[test]
fn slowing_a_link_raises_latency_by_exactly_the_added_delay() {
    const D_US: u64 = 500;
    let horizon = 12 * PERIOD_PS;
    let mut base = world(&[SOURCE, RELAY, RELAY], "");
    run_to(&mut base, horizon);
    let mut slow = world(
        &[SOURCE, RELAY, RELAY],
        "      events:\n        - { at_us: 5500, slow_link: 1, delay_us: 500 }\n",
    );
    run_to(&mut slow, horizon);
    let (rb, rs) = (report(&base), report(&slow));
    let d = D_US * 1_000_000;
    let mut slowed = 0;
    for m in &rs.messages {
        let (Some(lb), Some(ls)) = (latency(&rb, m.id, "n2"), latency(&rs, m.id, "n2")) else {
            continue;
        };
        // Link 1 changed at 5.5 ms. The delay applies to characters that
        // start later; the latency is set by a message's LAST character,
        // which starts 4 frames after its forward began (~origin + one hop
        // + p, p < 5 µs: never near the edge for these origins).
        let last_char_start = origin(&rs, m.id).unwrap() + hop_ps() + 4 * 10 * bit_ps();
        let expect = if last_char_start >= 5_500_000_000 {
            d
        } else {
            0
        };
        assert!(
            (ls - lb).abs_diff(expect) <= cycle_ps(),
            "msg {}: slowed-run latency {} vs baseline {} (predicted +{})",
            m.id,
            ls,
            lb,
            expect
        );
        assert_eq!(
            latency(&rb, m.id, "n1"),
            latency(&rs, m.id, "n1"),
            "link 0 untouched"
        );
        if expect > 0 {
            slowed += 1;
        }
    }
    assert!(
        slowed >= 4,
        "only {slowed} messages crossed the slowed link"
    );
    let l = &rs.links[1].directions[0].stats;
    eprintln!(
        "SLOW LINK: predicted +{D_US} us; measured +{:.3} us on {slowed} messages; max in flight on link 1: {}",
        ps_to_us(latency(&rs, 8, "n2").unwrap() - latency(&rb, 8, "n2").unwrap()),
        l.max_in_flight
    );
    // 500 µs of wire holds 500 / 86.8 = 5.76 characters: up to 6 in flight.
    assert!((5..=7).contains(&l.max_in_flight), "{}", l.max_in_flight);
}

/// Renode #948's test: link 1 slows down, then the middle node restarts while
/// it is forwarding. The chain loses only the message that was in the
/// restarted node's hands, then recovers, and no buffer overflows anywhere.
#[test]
fn slow_link_plus_node_restart_recovers_without_overflow() {
    let reset_us = 8_600; // n1 is mid-way through forwarding message 7
    let mut w = world(
        &[SOURCE, RELAY, RELAY],
        &format!(
            "      events:\n        - {{ at_us: 5500, slow_link: 1, delay_us: 500 }}\n        - {{ at_us: {reset_us}, reset_node: n1 }}\n"
        ),
    );
    run_to(&mut w, 16 * PERIOD_PS);
    let r = report(&w);
    let reset_ps = reset_us * 1_000_000;

    // Which messages can the reset destroy? Those that n1 holds at the reset:
    // received (fully or partly) but not fully sent. Predicted from origins.
    let predicted_lost: Vec<u32> = r
        .messages
        .iter()
        .filter(|m| m.origin_node.as_deref() == Some("n0"))
        .filter(|m| {
            let o = m.origin_ps.unwrap();
            // n1 holds it from its first character's arrival until it has
            // shifted out its own last character (one hop + p later).
            o < reset_ps && reset_ps < o + 2 * hop_ps() + 10_000_000
        })
        .map(|m| m.id)
        .collect();
    // Plus one: n1's truncated forward leaves n2's parser holding part of a
    // frame, and the fixture's parser has no inter-character timeout, so it
    // spends the NEXT message resynchronising (checksum failure). That is
    // the firmware's behaviour, and the timeline is what shows it.
    let mut predicted_lost = predicted_lost;
    predicted_lost.push(predicted_lost.last().unwrap() + 1);
    let at_n2 = ids_delivered_to(&r, "n2");
    let last_sent = r
        .messages
        .iter()
        .filter(|m| m.origin_node.as_deref() == Some("n0"))
        .map(|m| m.id)
        .max()
        .unwrap();
    let lost: Vec<u32> = (0..last_sent.saturating_sub(1))
        .filter(|id| !at_n2.contains(id))
        .collect();
    eprintln!(
        "RESET n1 at {reset_us} us: predicted lost {predicted_lost:?}, measured lost {lost:?}; delivered {at_n2:?}"
    );
    assert_eq!(
        lost, predicted_lost,
        "exactly the messages n1 held are lost"
    );
    assert!(lost.len() <= 2, "bounded loss");
    assert!(n2_checksum_failures(&w) >= 1);

    // Recovery: every message after the lost ones arrives, still slowed.
    let first_after = lost.last().unwrap() + 1;
    for id in first_after..last_sent.saturating_sub(1) {
        assert!(at_n2.contains(&id), "msg {id} lost after recovery");
        let l = latency(&r, id, "n2").unwrap();
        let p = l - 2 * hop_ps() - 500_000_000;
        assert!(p > 0 && p < 10_000_000, "msg {id}: p = {p}");
    }

    // The node really restarted, the timeline says when, and no buffer
    // overflowed: no overrun anywhere, rings never above one message.
    assert_eq!(boots(&w, "n1"), 2);
    assert_eq!(r.node_resets.get("n1"), Some(&1));
    assert!(r
        .events
        .iter()
        .any(|e| e.kind == NetEventKind::NodeReset && e.node == "n1" && e.t_ps == reset_ps));
    for link in &r.links {
        let s = &link.directions[0].stats;
        assert_eq!((s.overruns, s.late), (0, 0), "{s:?}");
    }
    for node in ["n1", "n2"] {
        let res = result_words(&w, node);
        assert_eq!(res[4], 0, "{node}: firmware saw ORE");
        assert!(
            res[6] <= MSG_LEN as u32,
            "{node}: RX ring high-water {}",
            res[6]
        );
        assert!(
            res[7] <= MSG_LEN as u32,
            "{node}: TX ring high-water {}",
            res[7]
        );
        assert_eq!(res[9], 0, "{node}: TX ring overflowed");
    }
    let n2 = result_words(&w, "n2");
    let s1 = &r.links[1].directions[0].stats;
    eprintln!(
        "AFTER RESET: n2 accepted {} (checksum failures {}, gaps {}); link 1 truncated {} dropped_rx_disabled {}; n1 boots {}",
        n2[0], n2[3], n2[10], s1.truncated, r.links[0].directions[0].stats.dropped_rx_disabled,
        boots(&w, "n1")
    );
    assert_eq!(n2[10], 1, "n2 saw exactly one gap in the sequence");
    assert_eq!(
        n2[3], 1,
        "n2's parser resynchronised once (one checksum failure)"
    );
    assert!(s1.truncated >= 1, "the reset cut n1's character short");
}

/// Cutting link 0 loses exactly the messages whose characters were due on it
/// while it was cut; the chain recovers on restore.
#[test]
fn a_cut_link_loses_exactly_the_messages_on_it_and_recovers() {
    let (cut_us, restore_us) = (3_200u64, 6_300u64);
    let mut w = world(
        &[SOURCE, RELAY, RELAY],
        &format!(
            "      events:\n        - {{ at_us: {cut_us}, cut_link: 0 }}\n        - {{ at_us: {restore_us}, restore_link: 0 }}\n"
        ),
    );
    run_to(&mut w, 12 * PERIOD_PS);
    let r = report(&w);
    let (cut, restore) = (cut_us * 1_000_000, restore_us * 1_000_000);
    let wire = (MSG_LEN - 1) * 10 * bit_ps() + 10 * bit_ps();
    let at_n2 = ids_delivered_to(&r, "n2");
    let mut predicted_lost = Vec::new();
    for m in r
        .messages
        .iter()
        .filter(|m| m.origin_node.as_deref() == Some("n0"))
    {
        let o = m.origin_ps.unwrap();
        if o < restore && o + wire > cut {
            predicted_lost.push(m.id);
        }
    }
    // The message cut mid-way left n1's parser holding part of a frame; with
    // no inter-character timeout it spends the first message after the
    // restore resynchronising (see the reset test).
    predicted_lost.push(predicted_lost.last().unwrap() + 1);
    let lost: Vec<u32> = (0..10).filter(|id| !at_n2.contains(id)).collect();
    eprintln!("CUT link 0 {cut_us}..{restore_us} us: predicted lost {predicted_lost:?}, measured {lost:?}");
    assert_eq!(lost, predicted_lost);
    assert!(r.links[0].directions[0].stats.dropped_link_cut > 0);
    assert!(r.events.iter().any(|e| e.kind == NetEventKind::LinkCut));
    assert!(r.events.iter().any(|e| e.kind == NetEventKind::LinkRestore));
}

/// An RX interrupt that spends 150 µs per character cannot keep up with one
/// character every 86.8 µs: the USART reports overrun (ORE), the firmware
/// sees it, and messages are lost.
#[test]
fn an_undersized_rx_path_shows_overrun() {
    let mut w = world(&[SOURCE, "uart-chain-relay-slow-rx.elf", RELAY], "");
    run_to(&mut w, 8 * PERIOD_PS);
    let r = report(&w);
    let s = &r.links[0].directions[0].stats;
    let n1 = result_words(&w, "n1");
    eprintln!(
        "SLOW RX: link 0 overruns {} of {} chars; firmware ORE count {}; n1 accepted {} (checksum failures {})",
        s.overruns, s.chars_sent, n1[4], n1[0], n1[3]
    );
    assert!(s.overruns > 0);
    assert!(n1[4] > 0, "the firmware's ISR saw SR.ORE");
    assert!(r.events.iter().any(|e| e.kind == NetEventKind::Overrun));
    // A 5-character message loses characters to overrun, so none survives
    // intact to n2.
    assert!(ids_delivered_to(&r, "n2").is_empty());
}

/// A relay programmed for 57600 baud on a 115200 line reads framing errors,
/// not clean bytes.
#[test]
fn a_baud_mismatch_gives_framing_errors_not_delivery() {
    let mut w = world(&[SOURCE, "uart-chain-relay-57600.elf", RELAY], "");
    run_to(&mut w, 6 * PERIOD_PS);
    let r = report(&w);
    let s = &r.links[0].directions[0].stats;
    let n1 = result_words(&w, "n1");
    eprintln!(
        "BAUD MISMATCH: {} chars sent at 115200, n1 at 57600: {} framing errors, {} false starts, n1 accepted {} messages (firmware FE count {})",
        s.chars_sent, s.framing_errors, s.false_starts, n1[0], n1[5]
    );
    assert!(s.framing_errors > 0);
    assert_eq!(n1[0], 0, "no message survives a baud mismatch");
    assert!(ids_delivered_to(&r, "n1").is_empty());
}

/// A star: hub n1 between spokes n0 (source) and n2. Physically the same path
/// as the chain, so the same latency.
#[test]
fn a_star_topology_carries_the_same_path_as_the_chain() {
    let mut star = world_with(
        &[SOURCE, RELAY, RELAY],
        "      hub: n1\n      hub_uarts: [uart1, uart2]\n      spoke_uarts: [uart2, uart1]\n",
        "star",
    );
    run_to(&mut star, 4 * PERIOD_PS);
    let mut chain = world(&[SOURCE, RELAY, RELAY], "");
    run_to(&mut chain, 4 * PERIOD_PS);
    let (rs, rc) = (report(&star), report(&chain));
    assert_eq!(rs.links.len(), 2);
    assert_eq!(latency(&rs, 0, "n2"), latency(&rc, 0, "n2"));
    assert!(latency(&rs, 0, "n2").is_some());
}

/// About ten nodes: the chain still runs, deterministically (two runs, and a
/// run with a much smaller synchronisation round, give the same timeline),
/// and the end-to-end latency is 9 hops + 8 relay times. Prints the
/// wall-clock cost.
#[test]
fn ten_node_chain_is_deterministic_and_prints_its_cost() {
    let fw: Vec<&str> = std::iter::once(SOURCE)
        .chain(std::iter::repeat_n(RELAY, 9))
        .collect();
    let horizon = 10 * PERIOD_PS;
    let run = |extra: &str| {
        let mut w = world(&fw, extra);
        let t = std::time::Instant::now();
        run_to(&mut w, horizon);
        (report(&w), t.elapsed())
    };
    let (a, ta) = run("");
    let (b, tb) = run("");
    let (c, tc) = run("      max_quantum_us: 3\n");
    // `seq` is recording order, which follows the round schedule; everything
    // physical (times, values, order by time) must match. A run stops at the
    // first round boundary past the horizon, so compare up to the horizon.
    let json = |r: &NetReport| {
        let events: Vec<_> = r
            .events
            .iter()
            .filter(|e| e.t_ps < horizon)
            .map(|e| {
                let mut e = e.clone();
                e.seq = 0;
                e
            })
            .collect();
        let messages: Vec<_> = r
            .messages
            .iter()
            .map(|m| {
                let mut m = m.clone();
                m.deliveries.retain(|d| d.t_ps < horizon);
                m
            })
            .filter(|m| m.origin_ps.is_some_and(|o| o < horizon))
            .collect();
        serde_json::to_string(&(&events, &messages)).unwrap()
    };
    let first_diff = |x: &str, y: &str| {
        let i = x.bytes().zip(y.bytes()).take_while(|(p, q)| p == q).count();
        let lo = i.saturating_sub(300);
        format!(
            "at byte {i}:\n  {}\n  {}",
            &x[lo..(i + 200).min(x.len())],
            &y[lo..(i + 200).min(y.len())]
        )
    };
    let (ja, jb, jc) = (json(&a), json(&b), json(&c));
    assert!(
        ja == jb,
        "two identical runs differ {}",
        first_diff(&ja, &jb)
    );
    assert!(
        ja == jc,
        "the round length changed the result {}",
        first_diff(&ja, &jc)
    );
    let l9 = latency(&a, 0, "n9").expect("msg 0 reached n9");
    let p = (l9 - 9 * hop_ps()) / 8;
    eprintln!(
        "TEN NODES: {} ms simulated per run, wall {:?} / {:?} / {:?} (3 us rounds); {} timeline events; msg 0 at n9 after {:.3} us = 9 x {:.3} us + 8 x {:.3} us",
        horizon / 1_000_000_000,
        ta,
        tb,
        tc,
        a.events.len(),
        ps_to_us(l9),
        ps_to_us(hop_ps()),
        ps_to_us(p)
    );
    assert!(p > 0 && p < 10_000_000);
    for link in &a.links {
        assert_eq!(link.directions[0].stats.late, 0);
        assert_eq!(link.directions[0].stats.overruns, 0);
    }
}

/// The shipped example (`examples/uart-relay-chain/env.yaml`) builds from its
/// manifest through the same path the CLI uses and shows the same recovery.
#[test]
fn the_example_environment_runs_from_its_manifest() {
    let path = root().join("examples/uart-relay-chain/env.yaml");
    let manifest = EnvironmentManifest::from_file(&path).expect("example manifest");
    let mut w = World::from_manifest(manifest, path.parent().unwrap()).expect("example world");
    run_to(&mut w, 16 * PERIOD_PS);
    let r = report(&w);
    assert_eq!(boots(&w, "n1"), 2);
    assert_eq!(result_words(&w, "n2")[10], 1, "one gap");
    assert_eq!(result_words(&w, "n2")[4], 0, "no overrun");
    assert!(r
        .events
        .iter()
        .any(|e| e.kind == NetEventKind::Marker && e.node == "n2"));
    let lost: Vec<u32> = (0..12)
        .filter(|id| !ids_delivered_to(&r, "n2").contains(id))
        .collect();
    assert_eq!(lost, vec![7, 8]);
}
