// A bus-resident GPIO device must drive pins on parts whose input word is
// READ-ONLY to firmware — not only on the STM32-style ports where a store to
// IDR happens to land.
//
// `DevicePins` exposes two seams. `drive_idr_bit` is an ordinary MMIO store to
// the input register; `drive_input_bit` is the external-world seam
// (`set_external_input`). They are NOT interchangeable:
//
//   STM32V2   IDR @0x10 — write_reg accepts the store, so either seam works.
//   EFR32s2   DIN @0x14 — write_reg drops it by design ("DIN is read-only for
//                         firmware — like silicon, a store to it is ignored;
//                         external input arrives via set_external_input").
//   SAM PORT  IN  @0x20 — same, read-only.
//
// A device that drives only `drive_idr_bit` is therefore silently inert on the
// second and third families. That is the costliest possible failure: attach
// succeeds, `list_inputs` advertises the channel, the stimulus reports applied,
// and the pin never moves — a stimulus that proves nothing.
//
// Measured 2026-09-05 on brd2709a (EFR32MG26): a rotary encoder read
// CLK=0 DT=0 at rest and after a 3-detent stimulus, while nucleo-f401re walked
// the Gray code correctly from the identical diagram. `gpio_devices_walk_free`
// gates the same failure reached through the TICK; this file gates it reached
// through the REGISTER LAYOUT.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::keypad::Keypad;
use labwired_core::peripherals::components::rotary_encoder::RotaryEncoder;
use labwired_core::peripherals::gpio::{GpioPort, GpioRegisterLayout};
use labwired_core::Bus;

/// A real BRD2709A port base (GPIOC) and its DIN, from `efr32mg26.yaml`.
const GPIOC: u64 = 0x4003_C090;
const DIN: u64 = GPIOC + 0x14;
const CPU_HZ: u64 = 78_000_000;

fn efr32_bus() -> SystemBus {
    let mut bus = SystemBus::empty();
    bus.add_peripheral(
        "gpioc",
        GPIOC,
        0x30,
        None,
        Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Efr32s2)),
    );
    bus
}

fn din_bit(bus: &SystemBus, bit: u8) -> u32 {
    (bus.read_u32(DIN).unwrap() >> bit) & 1
}

/// Advance the bus through the real per-cycle tick, as `Machine::advance` does.
fn tick(bus: &mut SystemBus, cycles: u64) {
    for c in 0..cycles {
        bus.set_current_cycle(c);
        let _ = bus.tick_peripherals_fully();
    }
}

/// A store to DIN is ignored — the premise the rest of this file rests on. If
/// this ever fails, EFR32 stopped modelling its input word as read-only and the
/// gates below would pass for the wrong reason.
#[test]
fn a_store_to_efr32_din_is_ignored() {
    let mut bus = efr32_bus();
    let _ = bus.write_u32(DIN, 0xFFFF);
    assert_eq!(
        bus.read_u32(DIN).unwrap(),
        0,
        "EFR32 DIN must stay read-only to firmware; if a store lands, these gates prove nothing"
    );
}

/// The encoder's documented rest state is (A,B) = (1,1) — both contacts
/// released. On a read-only input word that level can only arrive through
/// `drive_input_bit`.
#[test]
fn a_rotary_encoder_reaches_its_rest_state_on_a_read_only_input_word() {
    let mut bus = efr32_bus();
    bus.gpio_devices.push(Box::new(RotaryEncoder::new(
        "enc".into(),
        DIN,
        5, // PC05, the deck's encoder A
        DIN,
        7, // PC07, the deck's encoder B
        CPU_HZ,
    )));

    tick(&mut bus, 64);

    assert_eq!(
        (din_bit(&bus, 5), din_bit(&bus, 7)),
        (1, 1),
        "encoder must settle at its documented rest value (both released/high); \
         reading 0,0 is the pin never having been driven at all"
    );
}

/// Rest is not enough: a knob that is stuck high proves as little as one stuck
/// low. Turning it must actually move a contact.
#[test]
fn a_turned_rotary_encoder_moves_a_contact_on_a_read_only_input_word() {
    use labwired_core::sim_input::SimInput;

    let mut bus = efr32_bus();
    bus.gpio_devices.push(Box::new(RotaryEncoder::new(
        "enc".into(),
        DIN,
        5,
        DIN,
        7,
        CPU_HZ,
    )));
    tick(&mut bus, 64);
    assert_eq!((din_bit(&bus, 5), din_bit(&bus, 7)), (1, 1), "precondition");

    bus.gpio_devices_of_mut::<RotaryEncoder>()
        .next()
        .unwrap()
        .set_input("position", 3.0)
        .expect("position is a declared channel");

    // One detent is ~8 ms of contact bounce at this clock; walk well past it
    // and record whether either contact ever left its rest level.
    let mut moved = false;
    for c in 64..(CPU_HZ / 20) {
        bus.set_current_cycle(c);
        let _ = bus.tick_peripherals_fully();
        if (din_bit(&bus, 5), din_bit(&bus, 7)) != (1, 1) {
            moved = true;
            break;
        }
    }

    assert!(
        moved,
        "turning the shaft must move a contact; a stimulus that reports applied \
         while both pins hold rest is the failure this file exists for"
    );
}

/// The keypad drives its columns through the same seam and had the same defect.
#[test]
fn a_keypad_column_falls_on_a_read_only_input_word() {
    let mut bus = efr32_bus();
    // Rows on DOUT (an output the firmware drives), columns on DIN.
    let row_odr: [(u64, u8); 4] = std::array::from_fn(|r| (GPIOC + 0x10, r as u8));
    let col_idr: [(u64, u8); 4] = std::array::from_fn(|c| (DIN, (c + 8) as u8));
    bus.gpio_devices
        .push(Box::new(Keypad::new("kp".into(), row_odr, col_idr)));
    bus.gpio_devices_of_mut::<Keypad>()
        .next()
        .unwrap()
        .set_pressed(Some((2, 1)));

    // Select row 2 (active low), then let the tick service the matrix.
    bus.write_u32(GPIOC + 0x10, 0b1111u32 & !(1 << 2)).unwrap();
    tick(&mut bus, 8);

    // ⚠️ ASSERT THE HIGH COLUMN FIRST, AND ASSERT IT AT ALL. An undriven DIN
    // reads 0, and "pressed" also expects 0 — so a bare `column == 0` passes
    // just as happily when the keypad was never wired to anything. Caught by
    // negative control: with the fix reverted, the low-only assertion still
    // went green while both encoder gates went red.
    //
    // The idle columns are the real evidence here: they can only read 1 if
    // something drove them.
    assert_eq!(
        din_bit(&bus, 8),
        1,
        "an unpressed column must be driven HIGH; reading 0 is the matrix never \
         having been driven at all, which a low-only assertion cannot tell apart \
         from a correct press"
    );
    assert_eq!(din_bit(&bus, 9), 0, "the pressed key's column must fall");
}
