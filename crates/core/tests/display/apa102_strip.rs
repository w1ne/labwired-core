// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::peripherals::components::declarative_led_strip::apa102;
use labwired_core::peripherals::spi::SpiDevice;

fn send(dev: &mut dyn SpiDevice, bytes: &[u8]) {
    dev.cs_select();
    for &b in bytes {
        dev.transfer(b);
    }
    dev.cs_release();
}

#[test]
fn decodes_two_pixels_with_brightness() {
    let mut strip = apa102("PA4", 2);
    // start frame, then LED0 = red full bright, LED1 = green half bright.
    let frame = [
        0x00, 0x00, 0x00, 0x00, // start
        0xFF, 0x00, 0x00, 0xFF, // LED0: bright=31, B=0, G=0, R=255
        0xF0, 0x00, 0xFF, 0x00, // LED1: bright=16, B=0, G=255, R=0
        0xFF, 0xFF, 0xFF, 0xFF, // end
    ];
    send(&mut strip, &frame);
    let px = strip.pixels();
    assert_eq!(px.len(), 2);
    assert_eq!(px[0].wire, [255, 0, 0]);
    assert_eq!(px[0].brightness, 31);
    assert_eq!(px[1].wire, [0, 255, 0]);
    assert_eq!(px[1].brightness, 16);
}

#[test]
fn extra_frames_beyond_num_pixels_are_dropped() {
    let mut strip = apa102("PA4", 1);
    let frame = [
        0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, // LED0 red
        0xFF, 0xFF, 0x00, 0x00, // LED1 — beyond strip length
    ];
    send(&mut strip, &frame);
    assert_eq!(strip.pixels().len(), 1);
}

#[test]
fn short_frame_keeps_previous_pixels() {
    let mut strip = apa102("PA4", 2);
    let good = [
        0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00,
    ];
    send(&mut strip, &good);
    // A truncated garbage transaction must not blank the strip.
    send(&mut strip, &[0x12, 0x34]);
    assert_eq!(strip.pixels()[0].wire, [255, 0, 0]);
    assert_eq!(strip.pixels()[0].brightness, 31);
}

#[test]
fn miso_is_not_driven() {
    let mut strip = apa102("PA4", 1);
    strip.cs_select();
    assert_eq!(strip.transfer(0xAA), 0x00);
    strip.cs_release();
}
