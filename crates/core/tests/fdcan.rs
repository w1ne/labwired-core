// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Multi-node FDCAN over the `CanBus` interconnect: two STM32H5 FDCAN
//! instances attached to one virtual bus exchange classic and CAN-FD
//! frames. Register sequences follow the silicon-pinned model
//! (capture13, NUCLEO-H563ZI): fixed SRAMCAN layout, TX buffer 0 at
//! +0x278, RX FIFO0 element 0 at +0xB0.

use labwired_core::network::{CanBus, Interconnect};
use labwired_core::peripherals::fdcan::Fdcan;
use labwired_core::Peripheral;

/// SRAMCAN within the peripheral window.
const RAM: u64 = 0x800;
const REG_CCCR: u64 = 0x018;
const REG_RXF0S: u64 = 0x090;
const REG_TXBAR: u64 = 0x0CC;
const REG_TXBRP: u64 = 0x0C8;
const REG_TXBTO: u64 = 0x0D4;

fn rd(dev: &Fdcan, offset: u64) -> u32 {
    Peripheral::read_u32(dev, offset).unwrap()
}

fn wr(dev: &mut Fdcan, offset: u64, value: u32) {
    Peripheral::write_u32(dev, offset, value).unwrap()
}

/// Leave INIT so the protocol runs (reset state has INIT set).
fn start(dev: &mut Fdcan) {
    wr(dev, REG_CCCR, 0x0);
    assert_eq!(rd(dev, REG_CCCR) & 0x1, 0);
}

#[test]
fn fdcan_nodes_exchange_classic_can_frames_over_can_bus() {
    let mut can_bus = CanBus::new();
    let (tx_a, rx_a) = can_bus.attach();
    let (tx_b, rx_b) = can_bus.attach();
    let mut node_a = Fdcan::new_with_bus(tx_a, rx_a);
    let mut node_b = Fdcan::new_with_bus(tx_b, rx_b);

    // TX buffer 0 on node A: std ID 0x123, DLC 2.
    wr(&mut node_a, RAM + 0x278, 0x123 << 18);
    wr(&mut node_a, RAM + 0x27C, 2 << 16);
    wr(&mut node_a, RAM + 0x280, 0x0000_BBAA);
    start(&mut node_a);
    start(&mut node_b);

    wr(&mut node_a, REG_TXBAR, 0x1);
    // TX is asynchronous: node_a must tick first so drain_pending_tx
    // sends the frame to the bus channel, then the bus propagates it
    // to node_b on the subsequent ticks.
    node_a.tick();
    can_bus.tick().unwrap();
    node_b.tick();

    assert_eq!(rd(&node_b, REG_RXF0S) & 0x7F, 1, "one frame in FIFO0");
    assert_eq!((rd(&node_b, RAM + 0xB0) >> 18) & 0x7FF, 0x123);
    assert_eq!((rd(&node_b, RAM + 0xB4) >> 16) & 0xF, 2);
    assert_eq!(rd(&node_b, RAM + 0xB8) & 0xFFFF, 0xBBAA);
}

#[test]
fn fdcan_can_fd_frames_preserve_fd_metadata_and_64_byte_payload() {
    let mut can_bus = CanBus::new();
    let (tx_a, rx_a) = can_bus.attach();
    let (tx_b, rx_b) = can_bus.attach();
    let mut node_a = Fdcan::new_with_bus(tx_a, rx_a);
    let mut node_b = Fdcan::new_with_bus(tx_b, rx_b);

    // CAN-FD with bitrate switch: DLC 15 = 64 bytes, FDF (T1 bit 21),
    // BRS (T1 bit 20).
    wr(&mut node_a, RAM + 0x278, 0x456 << 18);
    wr(&mut node_a, RAM + 0x27C, (15 << 16) | (1 << 21) | (1 << 20));
    for i in 0..16u64 {
        wr(
            &mut node_a,
            RAM + 0x280 + i * 4,
            0x0302_0100 + (i as u32) * 0x0404_0404,
        );
    }
    start(&mut node_a);
    start(&mut node_b);

    wr(&mut node_a, REG_TXBAR, 0x1);
    // TX is asynchronous: tick node_a first to push the frame onto the bus.
    node_a.tick();
    can_bus.tick().unwrap();
    node_b.tick();

    assert_eq!(rd(&node_b, REG_RXF0S) & 0x7F, 1);
    assert_eq!((rd(&node_b, RAM + 0xB0) >> 18) & 0x7FF, 0x456);
    let r1 = rd(&node_b, RAM + 0xB4);
    assert_eq!((r1 >> 16) & 0xF, 15, "DLC 15 = 64 bytes");
    assert_ne!(r1 & (1 << 21), 0, "FDF survives the interconnect");
    assert_ne!(r1 & (1 << 20), 0, "BRS survives the interconnect");
    for i in 0..16u64 {
        assert_eq!(
            rd(&node_b, RAM + 0xB8 + i * 4),
            0x0302_0100 + (i as u32) * 0x0404_0404,
            "payload word {i}"
        );
    }
}

#[test]
fn transmitter_does_not_receive_its_own_frame_but_other_endpoints_do() {
    // A CAN bus is a shared medium, but a node must not feed its own TX frame
    // into its RX FIFO. The observer never transmits; it proves delivery is
    // addressed to each *other* endpoint rather than merely routed through a
    // second active transmitter.
    let mut can_bus = CanBus::new();
    let (tx_a, rx_a) = can_bus.attach();
    let (tx_b, rx_b) = can_bus.attach();
    let (tx_observer, rx_observer) = can_bus.attach();
    let mut node_a = Fdcan::new_with_bus(tx_a, rx_a);
    let mut node_b = Fdcan::new_with_bus(tx_b, rx_b);
    let mut observer = Fdcan::new_with_bus(tx_observer, rx_observer);

    wr(&mut node_a, RAM + 0x278, 0x321 << 18);
    wr(&mut node_a, RAM + 0x27C, 1 << 16);
    start(&mut node_a);
    start(&mut node_b);
    start(&mut observer);
    wr(&mut node_a, REG_TXBAR, 0x1);
    assert_eq!(rd(&node_a, REG_RXF0S) & 0x7F, 0, "no rx before node tick");

    // Tick node_a first so drain_pending_tx sends the frame to the bus
    // channel; then the bus fans out to every other endpoint.
    node_a.tick();
    can_bus.tick().unwrap();
    node_a.tick();
    node_b.tick();
    observer.tick();

    assert_eq!(
        rd(&node_a, REG_RXF0S) & 0x7F,
        0,
        "sender must not self-echo"
    );
    for (name, node) in [("receiver", &node_b), ("observer", &observer)] {
        assert_eq!(rd(node, REG_RXF0S) & 0x7F, 1, "{name} gets the frame");
        assert_eq!(
            (rd(node, RAM + 0xB0) >> 18) & 0x7FF,
            0x321,
            "{name} gets the exact standard identifier"
        );
    }
}

/// Issue #336: TXBRP must stay asserted until tick() completes the
/// transmission. Firmware that polls `while (TXBRP & 1) {}` should
/// actually block in simulation, preventing false-pass of code that
/// resets before the frame reaches the bus (the udslib #88 pattern).
#[test]
fn txbrp_stays_set_until_tick_then_txbto_is_set() {
    // Register offsets used in this test only.
    const REG_TEST: u64 = 0x010;

    let mut dev = Fdcan::new();

    // Enter loopback using the same sequence as capture13: CCCR.INIT +
    // CCCR.CCE + CCCR.TEST, then TEST.LBCK (bit 4), then leave INIT.
    wr(&mut dev, REG_CCCR, 0x3); // INIT | CCE
    wr(&mut dev, REG_CCCR, 0xA3); // + TEST | MON
    wr(&mut dev, REG_TEST, 1 << 4); // TEST.LBCK
    wr(&mut dev, REG_CCCR, 0xA2); // leave INIT, keep TEST | MON

    wr(&mut dev, RAM + 0x278, 0x100 << 18); // TX buf 0: std ID 0x100
    wr(&mut dev, RAM + 0x27C, 1 << 16); // DLC 1

    // Write TXBAR — frame is queued as pending, TXBRP asserted.
    wr(&mut dev, REG_TXBAR, 0x1);

    // TXBRP must be non-zero immediately after the TXBAR write.
    assert_ne!(
        rd(&dev, REG_TXBRP) & 0x1,
        0,
        "TXBRP must be set before tick (frame in flight)"
    );
    // TXBTO must still be zero — completion not yet posted.
    assert_eq!(
        rd(&dev, REG_TXBTO) & 0x1,
        0,
        "TXBTO must be clear before tick"
    );

    // One tick — drain_pending_tx delivers the frame and posts flags.
    dev.tick();

    // Completion flags are now visible to the firmware.
    assert_eq!(rd(&dev, REG_TXBRP) & 0x1, 0, "TXBRP must clear after tick");
    assert_ne!(rd(&dev, REG_TXBTO) & 0x1, 0, "TXBTO must set after tick");
    // Loopback path: frame must have reached RX FIFO0 on the same tick.
    assert_eq!(
        rd(&dev, REG_RXF0S) & 0x7F,
        1,
        "loopback frame in FIFO0 after tick"
    );
}
