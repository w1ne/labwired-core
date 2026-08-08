// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// SSD1306 128×64 OLED status display over nRF52 TWIM0 (I²C EasyDMA).
//
// The lab's "dashboard": every boot phase paints its verdict on the panel so
// the demo is visible in the browser playground / live dashboard, not just in
// uart.log. Protocol logic follows examples/ssd1306-hello-lab; the transport
// is the TWIM EasyDMA engine (one transfer per command, one big transfer for
// the full 1 KiB framebuffer flush).

use crate::font5x7::{FONT5X7, FONT_FIRST, FONT_LAST};
use crate::{rd, wr};

// TWIM0 (i2c0 serial window) register offsets — nRF52840 PS §6.31.
pub(crate) const TWIM0_BASE: usize = 0x4000_3000;
const TWIM_TASKS_STARTTX: usize = 0x008;
const TWIM_TASKS_STOP: usize = 0x014;
const TWIM_EVENTS_LASTTX: usize = 0x160;
const TWIM_ENABLE: usize = 0x500;
const TWIM_PSEL_SCL: usize = 0x508;
const TWIM_PSEL_SDA: usize = 0x50C;
const TWIM_FREQUENCY: usize = 0x524;
const TWIM_TXD_PTR: usize = 0x544;
const TWIM_TXD_MAXCNT: usize = 0x548;
const TWIM_ADDRESS: usize = 0x588;
const TWIM_ENABLE_TWIM: u32 = 6;
const TWIM_FREQ_K400: u32 = 0x0640_0000;

const OLED_ADDR: u32 = 0x3C;

// nRF52840 DK silkscreen I²C pins (informational — the sim does not gate the
// transfer on PSEL, but real silicon does, so set them correctly).
const PIN_SCL: u32 = 27;
const PIN_SDA: u32 = 26;

/// 128×64 monochrome framebuffer, page-major (8 pages × 128 columns).
static mut FB: [u8; 1024] = [0; 1024];
/// TX staging: 1 control byte + full framebuffer (EasyDMA reads RAM only).
static mut TXBUF: [u8; 1025] = [0; 1025];

unsafe fn twim_tx(buf: *const u8, len: usize) {
    // The bus is shared with the secure element, which sets its own ADDRESS
    // per transaction — re-select the panel every time.
    wr(TWIM0_BASE, TWIM_ADDRESS, OLED_ADDR);
    wr(TWIM0_BASE, TWIM_EVENTS_LASTTX, 0);
    wr(TWIM0_BASE, TWIM_TXD_PTR, buf as u32);
    wr(TWIM0_BASE, TWIM_TXD_MAXCNT, len as u32);
    wr(TWIM0_BASE, TWIM_TASKS_STARTTX, 1);
    while rd(TWIM0_BASE, TWIM_EVENTS_LASTTX) == 0 {}
}

/// Single SSD1306 command byte (control byte 0x00 = command stream).
unsafe fn oled_cmd(cmd: u8) {
    let buf = [0x00u8, cmd];
    twim_tx(buf.as_ptr(), 2);
}

/// Command byte followed by one parameter.
unsafe fn oled_cmd1(cmd: u8, p1: u8) {
    let buf = [0x00u8, cmd, p1];
    twim_tx(buf.as_ptr(), 3);
}

/// Command byte followed by two parameters.
unsafe fn oled_cmd2(cmd: u8, p1: u8, p2: u8) {
    let buf = [0x00u8, cmd, p1, p2];
    twim_tx(buf.as_ptr(), 4);
}

pub unsafe fn display_init() {
    wr(TWIM0_BASE, TWIM_PSEL_SCL, PIN_SCL);
    wr(TWIM0_BASE, TWIM_PSEL_SDA, PIN_SDA);
    wr(TWIM0_BASE, TWIM_FREQUENCY, TWIM_FREQ_K400);
    wr(TWIM0_BASE, TWIM_ADDRESS, OLED_ADDR);
    wr(TWIM0_BASE, TWIM_ENABLE, TWIM_ENABLE_TWIM);

    oled_cmd(0xAE); // display off
    oled_cmd1(0xD5, 0x80); // clock div / osc freq
    oled_cmd1(0xA8, 0x3F); // multiplex ratio 64
    oled_cmd1(0xD3, 0x00); // display offset 0
    oled_cmd(0x40); // start line 0
    oled_cmd1(0x8D, 0x14); // charge pump on
    oled_cmd1(0x20, 0x00); // horizontal addressing mode
    oled_cmd(0xA1); // segment remap
    oled_cmd(0xC8); // COM scan reversed
    oled_cmd1(0xDA, 0x12); // COM pins config
    oled_cmd1(0x81, 0xCF); // contrast
    oled_cmd1(0xD9, 0xF1); // pre-charge
    oled_cmd1(0xDB, 0x40); // VCOMH level
    oled_cmd(0xA4); // display from RAM
    oled_cmd(0xA6); // non-inverted
    oled_cmd(0xAF); // display on
}

/// Draw one text row (8 px tall) starting at pixel column `x` (6 px/char).
pub unsafe fn display_text(page: u8, x: usize, s: &str) {
    let fb = &mut *core::ptr::addr_of_mut!(FB);
    let base = page as usize * 128;

    // Blank the rest of the row first.
    //
    // The glyph loop below writes the 5 columns a character occupies and
    // nothing else, so a shorter string used to leave the tail of whatever was
    // on that row before. Boot 2 painted "ECDSA VERIFY..." on page 4 and then
    // "SE: SIG OK" over it, and the panel showed "SE: SIG OK" followed by a
    // leftover "FY...". Clearing only this row keeps the dashboard's other
    // lines, which the caller expects to survive.
    for c in x..128 {
        fb[base + c] = 0;
    }

    for (i, ch) in s.bytes().enumerate() {
        let col = x + i * 6;
        if col + 5 > 128 {
            break;
        }
        let mut c = ch;
        if c.is_ascii_lowercase() {
            c -= 32; // font is uppercase-only
        }
        if !(FONT_FIRST..=FONT_LAST).contains(&c) {
            c = b'?';
        }
        let glyph = &FONT5X7[(c - FONT_FIRST) as usize];
        for (g, &bits) in glyph.iter().enumerate() {
            fb[base + col + g] = bits;
        }
    }
}

/// Push the whole framebuffer to the panel (one 1025-byte EasyDMA transfer).
pub unsafe fn display_flush() {
    oled_cmd2(0x21, 0x00, 0x7F); // columns 0..127
    oled_cmd2(0x22, 0x00, 0x07); // pages 0..7
    let fb = &*core::ptr::addr_of!(FB);
    let tx = &mut *core::ptr::addr_of_mut!(TXBUF);
    tx[0] = 0x40; // control: data stream
    tx[1..].copy_from_slice(fb);
    twim_tx(tx.as_ptr(), tx.len());
    wr(TWIM0_BASE, TWIM_TASKS_STOP, 1);
}
