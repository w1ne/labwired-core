// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! ESP32-S3 "OpenAI deck" — a 10-key macro deck for the LabWired simulator.
//!
//! A 1.5" 128×128 SH1107 OLED (I²C0) renders a title bar and a 2×5 grid of key
//! slots; 10 momentary key switches sit on GPIO inputs. When a key is pressed
//! the firmware highlights that slot on the OLED and emits a host-protocol line
//! over USB-Serial-JTAG, e.g. `KEY3 PRESS action=SLOT3`. The host reads those
//! lines and maps each SLOTn to a real OpenAI action (filled in later).
//!
//! Runs identically on the simulator (SH1107 model attached to I²C0) and on
//! real silicon (SH1107 breakout + 10 switches wired to the GPIOs below).
//!
//! Pins (documented; mirror the esp32s3-i2c-tmp102 GPIO8/GPIO9 I²C choice):
//!   I²C0 SDA = GPIO8, SCL = GPIO9.
//!   KEY1..KEY10 = GPIO4, GPIO5, GPIO6, GPIO7, GPIO10, GPIO11, GPIO12, GPIO13,
//!                 GPIO14, GPIO15. (Strapping pins 0/3/45/46, USB 19/20 and the
//!                 SPI-flash pins 26–32 are deliberately avoided.)
//!
//! Keys are active-HIGH here (idle = low, pressed = high). This keeps the boot
//! state clean in the simulator — the S3 GPIO input register powers up at 0, so
//! no key reads as pressed until something drives it high. On real hardware wire
//! each switch between the GPIO and 3V3 with an external pull-down to ground.
//!
//! The firmware is FP-free: the simulator does not model the Xtensa FPU, so all
//! rendering is integer/bitmap work.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    gpio::{Input, InputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    main,
    Blocking,
};
use esp_println::println;

/// SH1107 7-bit I²C address. The simulator's `configure_xtensa_esp32s3` wiring
/// already pins an SSD1306 at 0x3C, and the ESP32-S3 I²C model dispatches a
/// transaction to the FIRST slave that matches the address — so the SH1107 is
/// attached at the kit's SA0=high address 0x3D to avoid that collision. On real
/// silicon, tie the panel's SA0 pin high to select 0x3D.
const OLED_ADDR: u8 = 0x3D;

const NUM_KEYS: usize = 10;
/// GPIO number backing each key, KEY1..KEY10. Kept in sync with system.yaml.
/// (Documentation-only: the `Input` pins below are constructed from the typed
/// `p.GPIOn` fields, which cannot be indexed by number.)
#[allow(dead_code)]
const KEY_PINS: [u8; NUM_KEYS] = [4, 5, 6, 7, 10, 11, 12, 13, 14, 15];

// ─── SH1107 low-level I²C helpers ───────────────────────────────────────────

/// Send a single command byte (control byte 0x00 = command stream).
fn oled_cmd(i2c: &mut I2c<'_, Blocking>, cmd: u8) {
    let _ = i2c.write(OLED_ADDR, &[0x00, cmd]);
}

/// Point the GDDRAM cursor at (page, col) using page-addressing commands.
///   page:  0xB0 | page          (page 0..15)
///   col:   0x00 | (col & 0x0F)  (lower nibble)
///          0x10 | (col >> 4)    (upper nibble, 7-bit column)
fn oled_set_pos(i2c: &mut I2c<'_, Blocking>, page: u8, col: u8) {
    oled_cmd(i2c, 0xB0 | (page & 0x0F));
    oled_cmd(i2c, 0x00 | (col & 0x0F));
    oled_cmd(i2c, 0x10 | (col >> 4));
}

/// Write a run of data bytes (control byte 0x40 = data stream) at (page, col).
/// `data` must be <= 16 bytes to stay well inside the S3 I²C TX FIFO (32 B).
fn oled_data(i2c: &mut I2c<'_, Blocking>, page: u8, col: u8, data: &[u8]) {
    oled_set_pos(i2c, page, col);
    let mut buf = [0x40u8; 17];
    let n = data.len().min(16);
    buf[1..1 + n].copy_from_slice(&data[..n]);
    let _ = i2c.write(OLED_ADDR, &buf[..1 + n]);
}

/// Fill `width` columns of `page` starting at `col0` with `byte`, chunked into
/// 16-column runs so no single I²C transaction overruns the FIFO.
fn oled_fill_page(i2c: &mut I2c<'_, Blocking>, page: u8, col0: u8, width: u8, byte: u8) {
    let fill = [byte; 16];
    let mut done = 0u8;
    while done < width {
        let n = core::cmp::min(16, width - done);
        oled_data(i2c, page, col0 + done, &fill[..n as usize]);
        done += n;
    }
}

/// Standard SH1107 128×128 initialisation sequence. Every honoured single-
/// parameter command (clock, contrast, multiplex, offset, start line, pre-
/// charge, VCOMH, DC-DC) is followed by its parameter byte.
fn oled_init(i2c: &mut I2c<'_, Blocking>) {
    oled_cmd(i2c, 0xAE); // display off
    oled_cmd(i2c, 0xD5);
    oled_cmd(i2c, 0x51); // clock divide / osc freq
    oled_cmd(i2c, 0x20); // page addressing mode
    oled_cmd(i2c, 0x81);
    oled_cmd(i2c, 0x4F); // contrast
    oled_cmd(i2c, 0xAD);
    oled_cmd(i2c, 0x8A); // DC-DC control on
    oled_cmd(i2c, 0xA8);
    oled_cmd(i2c, 0x7F); // multiplex ratio 128
    oled_cmd(i2c, 0xD3);
    oled_cmd(i2c, 0x60); // display offset
    oled_cmd(i2c, 0xDC);
    oled_cmd(i2c, 0x00); // display start line
    oled_cmd(i2c, 0xD9);
    oled_cmd(i2c, 0x22); // pre-charge
    oled_cmd(i2c, 0xDB);
    oled_cmd(i2c, 0x35); // VCOMH deselect
    oled_cmd(i2c, 0xA4); // resume to RAM content
    oled_cmd(i2c, 0xA6); // normal (non-inverted)
    oled_cmd(i2c, 0xAF); // display on
}

/// Clear all 16 pages of the panel.
fn oled_clear(i2c: &mut I2c<'_, Blocking>) {
    for page in 0..16u8 {
        oled_fill_page(i2c, page, 0, 128, 0x00);
    }
}

// ─── 5×7 font (column-major, bit0 = top row) ────────────────────────────────

/// Return the 5-column bitmap for a glyph. Covers the characters this UI draws
/// (A/C/D/E/I/K/N/O/P, digits, space); everything else renders as blank.
fn glyph(c: u8) -> [u8; 5] {
    match c {
        b'0' => [0x3E, 0x51, 0x49, 0x45, 0x3E],
        b'1' => [0x00, 0x42, 0x7F, 0x40, 0x00],
        b'2' => [0x42, 0x61, 0x51, 0x49, 0x46],
        b'3' => [0x21, 0x41, 0x45, 0x4B, 0x31],
        b'4' => [0x18, 0x14, 0x12, 0x7F, 0x10],
        b'5' => [0x27, 0x45, 0x45, 0x45, 0x39],
        b'6' => [0x3C, 0x4A, 0x49, 0x49, 0x30],
        b'7' => [0x01, 0x71, 0x09, 0x05, 0x03],
        b'8' => [0x36, 0x49, 0x49, 0x49, 0x36],
        b'9' => [0x06, 0x49, 0x49, 0x29, 0x1E],
        b'A' => [0x7E, 0x11, 0x11, 0x11, 0x7E],
        b'C' => [0x3E, 0x41, 0x41, 0x41, 0x22],
        b'D' => [0x7F, 0x41, 0x41, 0x22, 0x1C],
        b'E' => [0x7F, 0x49, 0x49, 0x49, 0x41],
        b'I' => [0x00, 0x41, 0x7F, 0x41, 0x00],
        b'K' => [0x7F, 0x08, 0x14, 0x22, 0x41],
        b'N' => [0x7F, 0x04, 0x08, 0x10, 0x7F],
        b'O' => [0x3E, 0x41, 0x41, 0x41, 0x3E],
        b'P' => [0x7F, 0x09, 0x09, 0x09, 0x06],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

/// Draw an ASCII string at (page, col) using the 5×7 font (6px per char).
fn oled_text(i2c: &mut I2c<'_, Blocking>, page: u8, col: u8, s: &[u8]) {
    let mut x = col;
    for &c in s {
        let g = glyph(c);
        oled_data(i2c, page, x, &g);
        oled_data(i2c, page, x + 5, &[0x00]); // 1px inter-char gap
        x += 6;
    }
}

// ─── Deck grid layout ───────────────────────────────────────────────────────

const CELL_W: u8 = 24; // columns per key cell
const GRID_COL0: u8 = 4; // left margin of the grid
/// Top page of the three-page-tall cell body for grid row 0 / row 1.
const ROW_TOP_PAGE: [u8; 2] = [4, 10];

/// Column and top page for key `idx` (0-based) in the 2×5 grid.
fn cell_origin(idx: usize) -> (u8, u8) {
    let row = idx / 5;
    let col = idx % 5;
    (GRID_COL0 + (col as u8) * CELL_W, ROW_TOP_PAGE[row])
}

/// Draw one key cell. `pressed` fills the cell solid (highlight); otherwise the
/// cell shows its 1-2 digit key number, centred, with a top/bottom rule.
fn draw_cell(i2c: &mut I2c<'_, Blocking>, idx: usize, pressed: bool) {
    let (col0, top) = cell_origin(idx);
    let body_w = CELL_W - 2; // 1px gutter each side
    if pressed {
        for p in 0..3u8 {
            oled_fill_page(i2c, top + p, col0 + 1, body_w, 0xFF);
        }
        return;
    }
    // Clear the three cell pages.
    for p in 0..3u8 {
        oled_fill_page(i2c, top + p, col0, CELL_W, 0x00);
    }
    // Top rule (bit0) and bottom rule (bit7) frame the cell.
    oled_fill_page(i2c, top, col0 + 1, body_w, 0x01);
    oled_fill_page(i2c, top + 2, col0 + 1, body_w, 0x80);

    // Centre the key number (1..10) on the middle page.
    let n = (idx + 1) as u8;
    if n < 10 {
        let x = col0 + (CELL_W - 5) / 2;
        oled_data(i2c, top + 1, x, &glyph(b'0' + n));
    } else {
        let x = col0 + (CELL_W - 11) / 2;
        oled_data(i2c, top + 1, x, &glyph(b'1'));
        oled_data(i2c, top + 1, x + 6, &glyph(b'0'));
    }
}

/// Paint the full deck UI: title, separator rule, and all 10 unpressed cells.
fn render_deck(i2c: &mut I2c<'_, Blocking>) {
    oled_clear(i2c);
    // Title "OPENAI DECK" centred on page 0 (11 chars × 6px = 66px → col 31).
    oled_text(i2c, 0, 31, b"OPENAI DECK");
    // Separator rule on page 2.
    oled_fill_page(i2c, 2, 4, 120, 0x10);
    for idx in 0..NUM_KEYS {
        draw_cell(i2c, idx, false);
    }
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());

    // I²C0 master on GPIO8 (SDA) / GPIO9 (SCL), mirroring esp32s3-i2c-tmp102.
    let mut i2c = I2c::new(p.I2C0, I2cConfig::default())
        .unwrap()
        .with_sda(p.GPIO8)
        .with_scl(p.GPIO9);

    // 10 key inputs, active-high (idle low). Order matches KEY_PINS / system.yaml.
    let cfg = InputConfig::default();
    let keys: [Input; NUM_KEYS] = [
        Input::new(p.GPIO4, cfg),
        Input::new(p.GPIO5, cfg),
        Input::new(p.GPIO6, cfg),
        Input::new(p.GPIO7, cfg),
        Input::new(p.GPIO10, cfg),
        Input::new(p.GPIO11, cfg),
        Input::new(p.GPIO12, cfg),
        Input::new(p.GPIO13, cfg),
        Input::new(p.GPIO14, cfg),
        Input::new(p.GPIO15, cfg),
    ];

    println!("openai-deck-s3: boot");
    oled_init(&mut i2c);
    render_deck(&mut i2c);
    println!("openai-deck-s3: OLED ready");

    // Two analog controls: a rotary knob (temperature) and a slide fader
    // (max-tokens), read once at startup off ADC1. Knob = GPIO1 (ADC1_CH0),
    // fader = GPIO2 (ADC1_CH1) — free pins (keys use 4-7/10-15, I²C uses 8/9).
    let mut adc_cfg = AdcConfig::new();
    let mut knob = adc_cfg.enable_pin(p.GPIO1, Attenuation::_11dB);
    let mut fader = adc_cfg.enable_pin(p.GPIO2, Attenuation::_11dB);
    let mut adc1 = Adc::new(p.ADC1, adc_cfg);
    let knob_raw: u16 = nb::block!(adc1.read_oneshot(&mut knob)).unwrap();
    let fader_raw: u16 = nb::block!(adc1.read_oneshot(&mut fader)).unwrap();
    // Map: knob → temperature 0.00..2.00; fader → max_tokens 0..4096.
    let temp_centi = (knob_raw as u32 * 200) / 4095;
    let max_tokens = (fader_raw as u32 * 4096) / 4095;
    println!(
        "PARAMS knob_raw={} fader_raw={} temp={}.{:02} max_tokens={}",
        knob_raw,
        fader_raw,
        temp_centi / 100,
        temp_centi % 100,
        max_tokens,
    );

    // Edge-detected polling loop. On a rising edge emit the host-protocol line
    // and highlight the cell; on a falling edge redraw the cell normally.
    let mut prev = [false; NUM_KEYS];
    loop {
        for i in 0..NUM_KEYS {
            let pressed = keys[i].is_high();
            if pressed != prev[i] {
                if pressed {
                    println!("KEY{} PRESS action=SLOT{}", i + 1, i + 1);
                } else {
                    println!("KEY{} RELEASE action=SLOT{}", i + 1, i + 1);
                }
                draw_cell(&mut i2c, i, pressed);
                prev[i] = pressed;
            }
        }
        core::hint::spin_loop();
    }
}
