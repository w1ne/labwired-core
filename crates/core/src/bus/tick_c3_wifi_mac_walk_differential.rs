use crate::bus::SystemBus;
use crate::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac;
use crate::{Bus, Peripheral};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

const MAC_BASE: u64 = 0x6003_3000;
const INTMATRIX: u64 = 0x600C_2000;
const LINE: u32 = 6;
const MAC_SOURCE: u32 = 0;

// wifi_mac register offsets / bits (private; mirrored for the test).
const RX_RING_BASE: u64 = 0x88;
const EVENT_CLR: u64 = 0xC40;
const PLCP0_AC0: u64 = 0xD08;
const TX_KICK_BITS: u32 = 0xC000_0000;
const EVENT_RX_DONE: u32 = 0x0100_4000;
const EVENT_TX_DONE: u32 = 0x80;
const DESC_OWNER: u32 = 1 << 31;
const DESC_EOF: u32 = 1 << 30;

// DRAM layout, inside the devkit 0x3FC8_0000 400KB SRAM AND the
// 0x3fc0_0000..0x3fd0_0000 window `deliver_one_rx` accepts.
const RX_DESC: u32 = 0x3FC8_1000;
const RX_BUF: u32 = 0x3FC8_1200;
const TX_DESC: u32 = 0x3FC8_2000;
const TX_BUF: u32 = 0x3FC8_2200;

/// A tiny "802.11" data frame (enough header for the medium/len parse).
fn tx_frame() -> Vec<u8> {
    let mut f = vec![0u8; 40];
    f[0] = 0x08; // data frame (type 2)
                 // addr2 (SA) bytes 10..16 — a nonzero source so the model is happy.
    f[10..16].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x02]);
    f
}

fn rx_frame() -> Vec<u8> {
    vec![0xB0u8, 0x00, 0xAA, 0xBB, 0xCC, 0xDD]
}

fn build_bus(scheduler: bool) -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root.join("../../configs/systems/esp32c3-devkit.yaml"))
            .expect("load esp32c3-devkit system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 devkit bus");
    bus.irq_fabric.esp32c3.routing = true;

    let idx = bus
        .find_peripheral_index_by_name("wifi_mac")
        .expect("devkit bus carries a wifi_mac");
    let mut dev = Esp32c3WifiMac::new();
    if scheduler {
        dev.attach_cycle_clock(bus.cycle_clock.clone());
    }
    bus.peripherals[idx].dev = Box::new(dev);
    bus.refresh_peripheral_index();

    // Route source 0 → line 6.
    bus.write_u32(INTMATRIX + MAC_SOURCE as u64 * 4, LINE)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
    bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();

    // Recompute walk-deletion over the swapped-in MAC: scheduler-driven ⇒
    // walk-deleted (the real deploy path); walk-driven ⇒ the walk stays live
    // so `tick()` re-emits the MAC level. (Without this the walk bus would
    // inherit `from_config`'s stale `true`, computed when the declarative
    // wifi_mac stub was still inert, and wrongly skip the walk.)
    bus.legacy_walk_disabled = bus.derive_walk_deletable();
    bus
}

fn wifi_mut(bus: &mut SystemBus) -> &mut Esp32c3WifiMac {
    let idx = bus.find_peripheral_index_by_name("wifi_mac").unwrap();
    bus.peripherals[idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Esp32c3WifiMac>()
        .unwrap()
}

#[derive(PartialEq, Debug)]
struct SessionResult {
    rx_desc_w0: u32,
    rx_buf_frame: Vec<u8>,
    event_word: u32,
    line_while_pending: u32,
    line_after_clear: u32,
    tx_frames: Vec<Vec<u8>>,
}

/// Drive one full WiFi session and capture every observable. Steps the bus
/// one cycle at a time exactly as `Machine::step` does: publish the cycle,
/// then run the peripheral tick only every `interval` cycles (the pump
/// processes one TX/RX per tick call). The pump runs inside
/// `tick_peripherals_with_costs` (the bus-tick pass), armed by the ring-enable
/// / TX-kick MMIO writes below.
fn run_session(scheduler: bool, interval: u32) -> SessionResult {
    let mut bus = build_bus(scheduler);

    // Lay out a 1-entry RX ring: an owner-held descriptor + its buffer.
    bus.write_u32(RX_DESC as u64, DESC_OWNER | 1600).unwrap(); // owner, cap 1600
    bus.write_u32(RX_DESC as u64 + 4, RX_BUF).unwrap();
    bus.write_u32(RX_DESC as u64 + 8, 0).unwrap();
    // Enable the RX ring (MMIO write → arms the pump via the write choke).
    bus.write_u32(MAC_BASE + RX_RING_BASE, RX_DESC).unwrap();

    // Lay out a TX lldesc (word1 = buffer ptr) + the frame in DRAM.
    let frame = tx_frame();
    bus.write_u32(TX_DESC as u64, 0).unwrap();
    bus.write_u32(TX_DESC as u64 + 4, TX_BUF).unwrap();
    for (i, b) in frame.iter().enumerate() {
        bus.write_u8(TX_BUF as u64 + i as u64, *b).unwrap();
    }

    // Queue a received frame (non-MMIO injection; the live ring keeps the MAC
    // resident so it is pumped without any extra re-arm).
    wifi_mut(&mut bus).queue_rx_frame(rx_frame());

    // Kick a TX: PLCP0 low-20 bits point at the TX lldesc, kick bits set.
    let plcp0 = TX_KICK_BITS | (TX_DESC & 0x000F_FFFF);
    bus.write_u32(MAC_BASE + PLCP0_AC0, plcp0).unwrap();

    // Step long enough to drain both rings (one TX + one RX, one per tick).
    for c in 1..=(interval as u64 * 8) {
        bus.set_current_cycle(c);
        if c % interval as u64 == 0 {
            bus.tick_peripherals_with_costs();
        }
    }

    let rx_desc_w0 = bus.read_u32(RX_DESC as u64).unwrap();
    let mut rx_buf_frame = vec![0u8; rx_frame().len()];
    for (i, b) in rx_buf_frame.iter_mut().enumerate() {
        // The rx-control header is 48 bytes; the 802.11 frame follows.
        *b = bus.read_u8(RX_BUF as u64 + 48 + i as u64).unwrap();
    }
    let event_word = bus.read_u32(MAC_BASE + 0xC3C).unwrap();
    let line_while_pending = bus.irq_fabric.esp32c3.irq_lines;
    let tx_frames = wifi_mut(&mut bus).take_tx_frames();

    // Acknowledge every event (W1C) and confirm the routed line drops.
    bus.write_u32(MAC_BASE + EVENT_CLR, event_word).unwrap();
    // On a walk bus (walk still live) one more tick re-derives the level;
    // on a scheduler/walk-deleted bus the write choke already did.
    bus.set_current_cycle(interval as u64 * 9);
    bus.tick_peripherals_with_costs();
    let line_after_clear = bus.irq_fabric.esp32c3.irq_lines;

    SessionResult {
        rx_desc_w0,
        rx_buf_frame,
        event_word,
        line_while_pending,
        line_after_clear,
        tx_frames,
    }
}

/// Non-vacuity: a captured session must actually have moved frames and
/// asserted the routed MAC line.
fn assert_non_vacuous(r: &SessionResult, what: &str) {
    assert_eq!(
        r.rx_desc_w0 & DESC_OWNER,
        0,
        "{what}: RX descriptor owner cleared (frame delivered)"
    );
    assert_ne!(r.rx_desc_w0 & DESC_EOF, 0, "{what}: RX descriptor EOF set");
    assert_eq!(
        r.rx_buf_frame,
        rx_frame(),
        "{what}: RX frame DMAed into the buffer"
    );
    assert_ne!(
        r.event_word & EVENT_RX_DONE,
        0,
        "{what}: RX-done event latched"
    );
    assert_ne!(
        r.event_word & EVENT_TX_DONE,
        0,
        "{what}: TX-done event latched"
    );
    assert_eq!(
        r.tx_frames.len(),
        1,
        "{what}: exactly one TX frame captured"
    );
    assert_eq!(
        r.line_while_pending,
        1 << LINE,
        "{what}: MAC level routed to the CPU line"
    );
    assert_eq!(
        r.line_after_clear, 0,
        "{what}: routed line de-asserts after EVENT_CLR"
    );
}

#[test]
fn wifi_session_walk_vs_scheduler_byte_identical_interval_1() {
    let walk = run_session(false, 1);
    let sched = run_session(true, 1);
    assert_non_vacuous(&walk, "walk@1");
    assert_eq!(
        walk, sched,
        "WiFi session must be byte-identical walk-vs-scheduler at interval 1"
    );
}

#[test]
fn wifi_session_walk_vs_scheduler_byte_identical_interval_64() {
    let walk = run_session(false, 64);
    let sched = run_session(true, 64);
    assert_non_vacuous(&sched, "sched@64");
    assert_eq!(
        walk, sched,
        "WiFi session must be byte-identical walk-vs-scheduler at interval 64"
    );
}

#[test]
fn wifi_session_scheduler_interval_independent() {
    // The pump's drained ring state + captured TX + event must not depend on
    // the tick interval (batching is fidelity-safe for the descriptor rings).
    let s1 = run_session(true, 1);
    let s64 = run_session(true, 64);
    assert_non_vacuous(&s1, "sched@1");
    assert_eq!(
        s1, s64,
        "scheduler WiFi session must be interval-independent (1 == 64)"
    );
}
