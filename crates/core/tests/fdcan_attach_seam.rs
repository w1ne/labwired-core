//! Post-construction FDCAN attachment is the seam `World::from_manifest`
//! needs after `SystemBus::from_config` has built a node's peripherals.

use labwired_core::bus::SystemBus;
use labwired_core::network::{CanBus, Interconnect};
use labwired_core::peripherals::fdcan::Fdcan;
use labwired_core::peripherals::uart::Uart;
use labwired_core::Peripheral;

const FDCAN_BASE: u64 = 0x4000_A400;
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

fn start(dev: &mut Fdcan) {
    wr(dev, REG_CCCR, 0);
}

#[test]
fn attach_can_bus_by_id_binds_a_constructed_fdcan_to_a_live_endpoint() {
    let mut source_bus = SystemBus::empty();
    source_bus.add_peripheral("fdcan1", FDCAN_BASE, 0x1000, None, Box::new(Fdcan::new()));

    let mut can_bus = CanBus::new();
    let (source_tx, source_rx) = can_bus.attach();
    source_bus
        .attach_can_bus_by_id("fdcan1", source_tx, source_rx)
        .expect("a named FDCAN accepts late CAN-bus binding");

    let (receiver_tx, receiver_rx) = can_bus.attach();
    let mut receiver = Fdcan::new_with_bus(receiver_tx, receiver_rx);
    start(&mut receiver);

    let source = source_bus.peripherals[0]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Fdcan>()
        .unwrap();
    wr(source, RAM + 0x278, 0x456 << 18);
    wr(source, RAM + 0x27C, 1 << 16);
    start(source);
    wr(source, REG_TXBAR, 1);
    source.tick();
    can_bus.tick().unwrap();
    receiver.tick();

    assert_eq!(rd(&receiver, REG_RXF0S) & 0x7F, 1);
    assert_eq!((rd(&receiver, RAM + 0xB0) >> 18) & 0x7FF, 0x456);
}

#[test]
fn attach_can_bus_by_id_rejects_missing_and_non_fdcan_peripherals() {
    let mut bus = SystemBus::empty();
    bus.add_peripheral("uart2", 0x4000_4400, 0x400, None, Box::new(Uart::new()));

    let mut can_bus = CanBus::new();
    let (missing_tx, missing_rx) = can_bus.attach();
    let missing = bus
        .attach_can_bus_by_id("does_not_exist", missing_tx, missing_rx)
        .unwrap_err()
        .to_string();
    assert_eq!(missing, "no peripheral 'does_not_exist'");

    let (wrong_tx, wrong_rx) = can_bus.attach();
    let wrong_kind = bus
        .attach_can_bus_by_id("uart2", wrong_tx, wrong_rx)
        .unwrap_err()
        .to_string();
    assert_eq!(wrong_kind, "peripheral 'uart2' is not an FDCAN");
}
