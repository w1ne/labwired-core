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
fn transmitter_echo_is_contained_to_the_bus_broadcast() {
    // The current CanBus broadcasts to every node including the sender
    // (documented deviation — a real CAN node does not receive its own
    // frame). This test pins the present behavior so a future
    // source-tagged network layer changes it deliberately.
    let mut can_bus = CanBus::new();
    let (tx_a, rx_a) = can_bus.attach();
    let mut node_a = Fdcan::new_with_bus(tx_a, rx_a);

    wr(&mut node_a, RAM + 0x278, 0x321 << 18);
    wr(&mut node_a, RAM + 0x27C, 1 << 16);
    start(&mut node_a);
    wr(&mut node_a, REG_TXBAR, 0x1);
    assert_eq!(rd(&node_a, REG_RXF0S) & 0x7F, 0, "no rx before bus tick");
    can_bus.tick().unwrap();
    node_a.tick();
    assert_eq!(rd(&node_a, REG_RXF0S) & 0x7F, 1, "broadcast echo");
    wr(&mut node_a, 0x094, 0);
    assert_eq!(rd(&node_a, REG_RXF0S) & 0x7F, 0);
}
