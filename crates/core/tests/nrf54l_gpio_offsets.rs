// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF54L GPIO port decodes its OWN compacted offsets.
//!
//! The nRF54L family keeps every nRF52 GPIO register and its meaning, but moves
//! the block: OUT 0x504 -> 0x000 and PIN_CNF 0x700 -> 0x080. Those are DIFFERENT
//! deltas (0x504 and 0x680), so no constant window offset serves both.
//!
//! Why this matters more than it looks: a port declared on the nRF52 map with a
//! shifted base still lights LEDs, because an LED needs only DIR and OUT, which
//! the constant shift happens to place correctly. What it drops is PIN_CNF —
//! and PIN_CNF is the ONLY register through which Zephyr and nrfx configure an
//! input's pull-up. A board whose buttons are `GPIO_PULL_UP | GPIO_ACTIVE_LOW`
//! would therefore boot, blink, and never read a press.

use labwired_core::peripherals::gpio::GpioPort;
use labwired_core::Peripheral;

/// PIN_CNF[n] on the nRF54L map, for a port with `pins` pins.
fn pin_cnf(n: u64) -> u64 {
    0x080 + 4 * n
}

fn write_u32(port: &mut GpioPort, offset: u64, value: u32) {
    for i in 0..4 {
        port.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)
            .unwrap();
    }
}

fn read_u32(port: &GpioPort, offset: u64) -> u32 {
    (0..4)
        .map(|i| (port.read(offset + i).unwrap() as u32) << (i * 8))
        .fold(0, |a, b| a | b)
}

/// PIN_CNF must reach the model, and its DIR bit must be the authoritative
/// direction — the path nrfx uses when it configures a pin without touching
/// the DIR register at all.
#[test]
fn nrf54l_pin_cnf_reaches_the_model_and_sets_direction() {
    let mut port = GpioPort::new_nrf54l(32);

    // Configure P1.22 (an nRF54LM20 DK LED) as an output through PIN_CNF only.
    write_u32(&mut port, pin_cnf(22), 0x0000_0001); // DIR = Output

    assert_eq!(
        read_u32(&port, 0x010) & (1 << 22),
        1 << 22,
        "PIN_CNF[22].DIR must be reflected in DIR at 0x010"
    );
    assert_eq!(
        read_u32(&port, pin_cnf(22)) & 1,
        1,
        "PIN_CNF[22] must read back what was written"
    );
}

/// The pull configuration a button needs, written where the driver writes it.
#[test]
fn nrf54l_pin_cnf_stores_the_pull_configuration() {
    let mut port = GpioPort::new_nrf54l(32);
    // P1.26 is DK button 0: input, PULL_UP (PULL field = 3 at bits 3:2).
    write_u32(&mut port, pin_cnf(26), 0b1100);
    assert_eq!(
        read_u32(&port, pin_cnf(26)) & 0b1100,
        0b1100,
        "PIN_CNF[26].PULL must survive the write"
    );
}

/// OUT / IN / DIR sit at the compacted offsets, not the nRF52 ones.
#[test]
fn nrf54l_out_and_dir_are_at_the_compacted_offsets() {
    let mut port = GpioPort::new_nrf54l(32);
    write_u32(&mut port, 0x014, 1 << 22); // DIRSET
    assert_eq!(read_u32(&port, 0x010) & (1 << 22), 1 << 22, "DIR @0x010");
    write_u32(&mut port, 0x004, 1 << 22); // OUTSET
    assert_eq!(read_u32(&port, 0x000) & (1 << 22), 1 << 22, "OUT @0x000");
}

/// NEGATIVE CONTROL — the layout is load-bearing.
///
/// The same PIN_CNF access decoded on the nRF52 map with the constant 0x504
/// shift the previous nRF54L profile uses lands on offset 0x584, which is not a
/// register, and is dropped. Without this, the tests above would pass for a
/// port that had simply been given a wider constant offset.
#[test]
fn the_constant_shift_arrangement_drops_pin_cnf() {
    // 0x504 is what `base = MDK_base - 0x504` produces for a window-relative
    // access: the model sees the real offset plus 0x504.
    let mut shifted = GpioPort::new_nrf52(32).with_window_offset(0x504);
    write_u32(&mut shifted, pin_cnf(22), 0x0000_0001);
    assert_eq!(
        read_u32(&shifted, 0x010 + 0x504 - 0x504) & (1 << 22),
        0,
        "a constant 0x504 shift cannot deliver PIN_CNF; if this ever passes, \
         the shift arrangement has become viable and this layout can be retired"
    );

    // And the correct layout DOES deliver it — the two halves of the control.
    let mut correct = GpioPort::new_nrf54l(32);
    write_u32(&mut correct, pin_cnf(22), 0x0000_0001);
    assert_eq!(read_u32(&correct, 0x010) & (1 << 22), 1 << 22);
}

/// Port width is enforced: nRF54LM20A P0 has 10 pins, and a write above that
/// must not invent one.
#[test]
fn nrf54l_port_width_is_enforced() {
    let mut p0 = GpioPort::new_nrf54l(10);
    write_u32(&mut p0, 0x014, 1 << 20); // DIRSET on a pin P0 does not have
    assert_eq!(
        read_u32(&p0, 0x010) & (1 << 20),
        0,
        "a 10-pin port must not accept a direction for pin 20"
    );
}
