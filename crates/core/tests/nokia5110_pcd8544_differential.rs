// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Display fidelity for the Nokia 5110 (PCD8544) panel shipped by the
//! `nokia5110-invaders-lab` — the panel used beyond the C3 SSD1306.
//!
//! The PCD8544 frames command vs pixel bytes by a dedicated D/C GPIO line, not
//! by byte semantics. This drives the component exactly as the bus does — latch
//! D/C low for commands, high for pixel data — programming the X/Y address
//! pointer and streaming pixel bytes, then asserts the panel's DDRAM renders
//! EXACTLY the expected content at the addressed cells (and that the
//! column-first auto-advance walks to the next cell). An addressing or
//! data-routing regression fails here.
//!
//! Named `*_pcd8544_differential` so the board-coverage ratchet discovers it.

use labwired_core::peripherals::components::Pcd8544;
use labwired_core::peripherals::spi::SpiDevice;

const WIDTH: usize = 84;

fn cmd(p: &mut Pcd8544, byte: u8) {
    p.set_dc_level(false);
    p.transfer(byte);
}

fn data(p: &mut Pcd8544, byte: u8) {
    p.set_dc_level(true);
    p.transfer(byte);
}

/// A pixel byte written at an addressed (bank, column) lands at exactly that
/// DDRAM cell, and the column-first pointer auto-advances to the next column.
#[test]
fn pcd8544_addressed_pixel_write_renders_expected_cells() {
    let mut p = Pcd8544::new("PB6".into(), "PC7".into());

    assert!(
        p.framebuffer().iter().all(|&b| b == 0),
        "DDRAM must be blank before any write"
    );

    // Set X = column 5, Y = bank 2 (basic instruction set, H = 0).
    cmd(&mut p, 0x80 | 5); // set X address
    cmd(&mut p, 0x40 | 2); // set Y address (bank)

    // Stream three deterministic pixel columns.
    data(&mut p, 0xAA);
    data(&mut p, 0x3C);
    data(&mut p, 0xFF);

    let fb = p.framebuffer();
    assert_eq!(fb.len(), WIDTH * 6, "PCD8544 DDRAM is 84x6 = 504 bytes");
    assert_eq!(fb[2 * WIDTH + 5], 0xAA, "column 5 of bank 2");
    assert_eq!(fb[2 * WIDTH + 6], 0x3C, "auto-advanced to column 6");
    assert_eq!(fb[2 * WIDTH + 7], 0xFF, "auto-advanced to column 7");
    assert!(
        fb.iter().filter(|&&b| b != 0).count() == 3,
        "exactly the three written columns are non-blank"
    );
}

/// The function-set command routes the extended instruction set (contrast /
/// bias) away from DDRAM, and a following command re-selects the basic set so
/// pixel writes resume — i.e. command framing is honoured, not written as data.
#[test]
fn pcd8544_command_bytes_do_not_leak_into_framebuffer() {
    let mut p = Pcd8544::new("PB6".into(), "PC7".into());

    // Enter extended set (H = 1) and program Vop/bias — none of this is pixels.
    cmd(&mut p, 0x21); // function set: H = 1
    cmd(&mut p, 0x80 | 0x3F); // set Vop (contrast)
    cmd(&mut p, 0x14); // bias system
    cmd(&mut p, 0x20); // function set: back to basic (H = 0)
    cmd(&mut p, 0x0C); // display control: normal mode

    assert!(
        p.framebuffer().iter().all(|&b| b == 0),
        "command bytes must not appear in DDRAM"
    );

    // Now a real pixel write at the origin lands.
    cmd(&mut p, 0x80); // X = 0
    cmd(&mut p, 0x40); // Y = 0
    data(&mut p, 0x5A);
    assert_eq!(
        p.framebuffer()[0],
        0x5A,
        "pixel write resumes after commands"
    );
}
