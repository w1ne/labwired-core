// A bus-resident GPIO device must keep working when the legacy walk is
// deleted.
//
// `service_gpio_devices()` — the pass that lets a keypad, rotary encoder or
// DHT22 drive the pins the firmware samples — runs inside the per-cycle tick.
// That tick has a fast path for buses with no orchestration work, and its
// predicate did not count `gpio_devices`, so on a walk-deleted bus the tick
// early-returned before the pass ever ran.
//
// The failure mode is the one that costs the most trust: attach succeeds,
// `list_inputs` advertises the channel, `set_input` returns Ok, and the pin
// never moves. A stimulus reports success and proves nothing.
//
// Every existing keypad/encoder test calls `service_gpio_devices()` directly,
// which is why this hid: they prove the DEVICE works, never that the tick
// calls it. These drive the real tick entry point instead.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::keypad::Keypad;
use labwired_core::peripherals::gpio::{GpioPort, GpioRegisterLayout};
use labwired_core::Bus;

const GPIOA: u64 = 0x4800_0000;
const GPIOB: u64 = 0x4800_0400;
const ODR: u64 = GPIOA + 0x14;
const IDR: u64 = GPIOB + 0x10;

/// Two GPIO ports and a 4x4 keypad, rows on GPIOA's ODR, columns on GPIOB's IDR.
fn bus_with_keypad(walk_deleted: bool) -> SystemBus {
    let mut bus = SystemBus::empty();
    bus.add_peripheral(
        "gpioa",
        GPIOA,
        0x400,
        None,
        Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
    );
    bus.add_peripheral(
        "gpiob",
        GPIOB,
        0x400,
        None,
        Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
    );
    let row_odr: [(u64, u8); 4] = std::array::from_fn(|r| (ODR, r as u8));
    let col_idr: [(u64, u8); 4] = std::array::from_fn(|c| (IDR, c as u8));
    bus.gpio_devices
        .push(Box::new(Keypad::new("kp".into(), row_odr, col_idr)));
    bus.legacy_walk_disabled = walk_deleted;
    bus
}

/// Scan the matrix through the REAL per-cycle tick and return the key found.
fn scan_via_tick(bus: &mut SystemBus) -> Option<(u8, u8)> {
    let mut found = None;
    for row in 0..4u8 {
        bus.write_u32(ODR, 0b1111u32 & !(1 << row)).unwrap();
        bus.set_current_cycle(row as u64);
        // The entry point Machine::advance uses — NOT service_gpio_devices().
        let _ = bus.tick_peripherals_fully();
        for col in 0..4u8 {
            if (bus.read_u32(IDR).unwrap() >> col) & 1 == 0 {
                found = Some((row, col));
            }
        }
    }
    found
}

/// Baseline: with the walk alive, the tick services the keypad.
#[test]
fn a_keypad_is_scannable_through_the_tick_on_a_walking_bus() {
    let mut bus = bus_with_keypad(false);
    bus.gpio_devices_of_mut::<Keypad>()
        .next()
        .unwrap()
        .set_pressed(Some((2, 1)));
    assert_eq!(scan_via_tick(&mut bus), Some((2, 1)));
}

/// The regression: deleting the walk must not silently un-wire the keypad.
///
/// Walk deletion is a PERFORMANCE decision about peripheral orchestration. A
/// device that drives a pin is not orchestration, and no perf path may decide
/// it stops existing.
#[test]
fn a_keypad_is_still_scannable_through_the_tick_on_a_walk_deleted_bus() {
    let mut bus = bus_with_keypad(true);
    bus.gpio_devices_of_mut::<Keypad>()
        .next()
        .unwrap()
        .set_pressed(Some((2, 1)));
    assert_eq!(
        scan_via_tick(&mut bus),
        Some((2, 1)),
        "walk deletion must not stop the tick from servicing bus-resident GPIO devices"
    );
}
