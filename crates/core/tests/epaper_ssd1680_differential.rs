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
//! `power_on()` and bumps `refresh_generation()`. A window/counter, plane-
//! routing, or refresh-sequence regression fails here.
//!
//! Named `*_ssd1680_differential` so the board-coverage ratchet discovers it.

use labwired_core::peripherals::components::Ssd1680Tricolor290;

const WIDTH_BYTES: usize = 16; // 128 native px / 8

/// A windowed black+red stream lands the exact bytes in each plane at the
/// windowed row cells (X-major auto-advance), leaving all other cells at the
/// erased default.
#[test]
fn ssd1680_windowed_stream_renders_both_planes() {
    let mut panel = Ssd1680Tricolor290::new("GPIO5").with_dc_pin("GPIO17");

    // Fresh panel: both planes erased (0xFF = white / no-red).
    assert!(panel.black_plane().iter().all(|&b| b == 0xFF));
    assert!(panel.red_plane().iter().all(|&b| b == 0xFF));

    // Reset + data-entry mode 0x03 (X+/Y+), then a 1-byte-wide, 4-row window.
    panel.command_byte(0x12); // SWRESET
    panel.command_byte(0x11); // data entry mode
    panel.data_byte(0x03);
    panel.command_byte(0x44); // RAM-X window: start/8, end/8
    panel.data_byte(0x00);
    panel.data_byte(0x00);
    panel.command_byte(0x45); // RAM-Y window: start_lo/hi, end_lo/hi
    panel.data_byte(0x00);
    panel.data_byte(0x00);
    panel.data_byte(0x03);
    panel.data_byte(0x00);
    panel.command_byte(0x4E); // RAM-X counter
    panel.data_byte(0x00);
    panel.command_byte(0x4F); // RAM-Y counter
    panel.data_byte(0x00);
    panel.data_byte(0x00);

    // Black plane: 4 bytes (window is 1 byte wide x 4 rows).
    let black: [u8; 4] = [0x00, 0xF0, 0x0F, 0xAA];
    panel.command_byte(0x24);
    for &b in &black {
        panel.data_byte(b);
    }
    // Red plane: same window.
    let red: [u8; 4] = [0xFF, 0x81, 0x18, 0x55];
    panel.command_byte(0x26);
    for &b in &red {
        panel.data_byte(b);
    }

    // Cells land at row-major idx = row * WIDTH_BYTES + col_byte (col 0).
    for (row, (&b, &r)) in black.iter().zip(red.iter()).enumerate() {
        let idx = row * WIDTH_BYTES;
        assert_eq!(panel.black_plane()[idx], b, "black plane row {row}");
        assert_eq!(panel.red_plane()[idx], r, "red plane row {row}");
    }

    // Everything outside the 4 windowed cells stays at the erased default.
    let touched_black = black.iter().filter(|&&b| b != 0xFF).count();
    assert_eq!(
        panel.black_plane().iter().filter(|&&b| b != 0xFF).count(),
        touched_black,
        "only windowed black cells differ from the erased default"
    );
}

/// The GxEPD2 power/refresh handshake: 0x22 selector 0xF8 powers the panel on,
/// and each 0x20 master activation advances the refresh generation.
#[test]
fn ssd1680_power_and_refresh_sequence() {
    let mut panel = Ssd1680Tricolor290::new("GPIO5").with_dc_pin("GPIO17");
    assert!(!panel.power_on(), "panel starts powered off");
    assert_eq!(panel.refresh_generation(), 0);

    // 0x22 = 0xF8 (power-on-only), then 0x20 activates it.
    panel.command_byte(0x22);
    panel.data_byte(0xF8);
    panel.command_byte(0x20);
    assert!(panel.power_on(), "0x22=0xF8 + 0x20 powers the panel on");
    assert_eq!(panel.refresh_generation(), 1);

    // A full-update refresh (0x22 = 0xF7) advances the generation again.
    panel.command_byte(0x22);
    panel.data_byte(0xF7);
    panel.command_byte(0x20);
    assert_eq!(
        panel.refresh_generation(),
        2,
        "each master activation bumps the refresh generation"
    );

    // 0x22 = 0x83 powers the panel back off.
    panel.command_byte(0x22);
    panel.data_byte(0x83);
    panel.command_byte(0x20);
    assert!(!panel.power_on(), "0x22=0x83 powers the panel off");
    assert_eq!(panel.refresh_generation(), 3);
}
