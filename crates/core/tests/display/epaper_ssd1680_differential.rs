// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Display fidelity for the SSD1680 tri-color 2.9" e-paper panel shipped by the
//! `esp32-epaper-lab` / `epaper-tricolor-lab` — a display beyond the C3
//! SSD1306, with independent black and red planes.
//!
//! GxEPD2 always configures the RAM window (0x44/0x45) and address counters
//! (0x4E/0x4F) before opening a 0x24 (black) or 0x26 (red) pixel stream. This
//! drives that exact datasheet sequence through the component's command/data
//! path and asserts each plane renders EXACTLY the streamed bytes at the
//! windowed cells, that the X-major auto-advance walks rows correctly, and that
//! the power/refresh sequence (0x22 selector + 0x20 master activation) toggles
//! the booster flag and bumps `refresh_generation()`. A window/counter, plane-
//! routing, or refresh-sequence regression fails here.
//!
//! Named `*_ssd1680_differential` so the board-coverage ratchet discovers it.
//!
//! The panel is a YAML `display` descriptor now, so the bytes go in through the
//! REAL `SpiDevice` door with a D/C level latched either side of each opcode —
//! the same path the bus drives — rather than through the deleted model's
//! `command_byte` / `data_byte` pair.

use labwired_core::peripherals::components::{ssd1680_tricolor_290, GenericDisplay};
use labwired_core::peripherals::spi::SpiDevice;

const WIDTH_BYTES: usize = 16; // 128 native px / 8

/// A panel with a RESOLVED D/C line, so framing comes off the latched level.
fn panel() -> GenericDisplay {
    let mut dev = ssd1680_tricolor_290("GPIO5");
    dev.set_dc_pin("GPIO17");
    SpiDevice::set_dc_source(&mut dev, 0x3FF4_4004, 17);
    dev
}

/// One opcode, D/C low.
fn command_byte(dev: &mut GenericDisplay, byte: u8) {
    dev.set_dc_level(false);
    dev.transfer(byte);
}

/// One parameter or pixel byte, D/C high.
fn data_byte(dev: &mut GenericDisplay, byte: u8) {
    dev.set_dc_level(true);
    dev.transfer(byte);
}

fn black_plane(dev: &GenericDisplay) -> Vec<u8> {
    dev.planes().ram("black").expect("black plane").to_vec()
}

fn red_plane(dev: &GenericDisplay) -> Vec<u8> {
    dev.planes().ram("red").expect("red plane").to_vec()
}

/// A windowed black+red stream lands the exact bytes in each plane at the
/// windowed row cells (X-major auto-advance), leaving all other cells at the
/// erased default.
#[test]
fn ssd1680_windowed_stream_renders_both_planes() {
    let mut panel = panel();

    // Fresh panel: both planes erased (0xFF = white / no-red).
    assert!(black_plane(&panel).iter().all(|&b| b == 0xFF));
    assert!(red_plane(&panel).iter().all(|&b| b == 0xFF));

    // Reset + data-entry mode 0x03 (X+/Y+), then a 1-byte-wide, 4-row window.
    command_byte(&mut panel, 0x12); // SWRESET
    command_byte(&mut panel, 0x11); // data entry mode
    data_byte(&mut panel, 0x03);
    command_byte(&mut panel, 0x44); // RAM-X window: start/8, end/8
    data_byte(&mut panel, 0x00);
    data_byte(&mut panel, 0x00);
    command_byte(&mut panel, 0x45); // RAM-Y window: start_lo/hi, end_lo/hi
    data_byte(&mut panel, 0x00);
    data_byte(&mut panel, 0x00);
    data_byte(&mut panel, 0x03);
    data_byte(&mut panel, 0x00);
    command_byte(&mut panel, 0x4E); // RAM-X counter
    data_byte(&mut panel, 0x00);
    command_byte(&mut panel, 0x4F); // RAM-Y counter
    data_byte(&mut panel, 0x00);
    data_byte(&mut panel, 0x00);

    // Black plane: 4 bytes (window is 1 byte wide x 4 rows).
    let black: [u8; 4] = [0x00, 0xF0, 0x0F, 0xAA];
    command_byte(&mut panel, 0x24);
    for &b in &black {
        data_byte(&mut panel, b);
    }
    // Red plane: same window.
    let red: [u8; 4] = [0xFF, 0x81, 0x18, 0x55];
    command_byte(&mut panel, 0x26);
    for &b in &red {
        data_byte(&mut panel, b);
    }

    // Cells land at row-major idx = row * WIDTH_BYTES + col_byte (col 0).
    for (row, (&b, &r)) in black.iter().zip(red.iter()).enumerate() {
        let idx = row * WIDTH_BYTES;
        assert_eq!(black_plane(&panel)[idx], b, "black plane row {row}");
        assert_eq!(red_plane(&panel)[idx], r, "red plane row {row}");
    }

    // Everything outside the 4 windowed cells stays at the erased default.
    let touched_black = black.iter().filter(|&&b| b != 0xFF).count();
    assert_eq!(
        black_plane(&panel).iter().filter(|&&b| b != 0xFF).count(),
        touched_black,
        "only windowed black cells differ from the erased default"
    );
}

/// The GxEPD2 power/refresh handshake: 0x22 selector 0xF8 powers the panel on,
/// and each 0x20 master activation advances the refresh generation.
#[test]
fn ssd1680_power_and_refresh_sequence() {
    let mut panel = panel();
    assert!(!panel.display_on(), "panel starts powered off");
    assert_eq!(panel.refresh_generation(), 0);

    // 0x22 = 0xF8 (power-on-only), then 0x20 activates it.
    command_byte(&mut panel, 0x22);
    data_byte(&mut panel, 0xF8);
    command_byte(&mut panel, 0x20);
    assert!(panel.display_on(), "0x22=0xF8 + 0x20 powers the panel on");
    assert_eq!(panel.refresh_generation(), 1);

    // A full-update refresh (0x22 = 0xF7) advances the generation again.
    command_byte(&mut panel, 0x22);
    data_byte(&mut panel, 0xF7);
    command_byte(&mut panel, 0x20);
    assert_eq!(
        panel.refresh_generation(),
        2,
        "each master activation bumps the refresh generation"
    );

    // 0x22 = 0x83 powers the panel back off.
    command_byte(&mut panel, 0x22);
    data_byte(&mut panel, 0x83);
    command_byte(&mut panel, 0x20);
    assert!(!panel.display_on(), "0x22=0x83 powers the panel off");
    assert_eq!(panel.refresh_generation(), 3);
}
