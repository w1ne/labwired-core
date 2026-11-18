// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Sitronix ST7920 graphic LCD controller (128x64 modules such as the
//! "LCD12864" and the RepRapDiscount Full Graphic Smart Controller) in serial
//! interface mode (PSB = 0).
//!
//! What the datasheet defines and this model follows:
//!
//! - **Serial framing.** Each transfer starts with a synchronising byte of five
//!   1s, RW, RS and a 0 (`0xF8 | RW << 2 | RS << 1`), then every 8-bit
//!   instruction or datum as two bytes: the upper nibble in D7..D4, then the
//!   lower nibble in D7..D4, D3..D0 zero. Bytes keep flowing under the last
//!   sync until the next one, so a driver can send one RS = 1 sync and a whole
//!   row of data. CS is active HIGH; CS low resets the serial counter. Serial
//!   mode has no read path, so MISO idles at 0.
//! - **Two instruction sets** selected by the RE bit of function set: basic
//!   (clear, home, entry mode, display on/off, cursor/display shift, function
//!   set, CGRAM address, DDRAM address) and extended (standby, scroll/RAM
//!   select, reverse, sleep, extended function set with the G bit, IRAM/scroll
//!   address, GDRAM address).
//! - **Text mode.** DDRAM holds 16-bit words, high byte first; the address
//!   counter steps after the second byte (by I/D). Codes 02H..7FH are
//!   half-width 16x8 HCGROM characters, two per word; 0000H/0002H/0004H/0006H
//!   select one of four 16x16 CGRAM glyphs (bits 1..2; bits 0 and 3 don't
//!   care); a high byte of A1H or above is a full-width 16x16 CGROM code.
//! - **Graphics mode.** GDRAM is addressed vertical-then-horizontal in 16-bit
//!   words, bit 15 the leftmost pixel. Only the horizontal address increments,
//!   wrapping 0FH -> 00H with no vertical carry.
//! - **128x64 folding.** The controller's address space is 256 pixels wide by
//!   32 rows; a 128x64 module shows its right half (horizontal words 8..15,
//!   DDRAM 08H..0FH and 18H..1FH) on rows 32..63.
//!
//! The published picture is the two layers combined per pixel with XOR (a
//! text pixel under a lit graphics pixel goes dark), which is how ST7920
//! modules are observed to mix the layers.
//!
//! The artifact is a `framebuffer` in the 1-bpp page layout the SSD1306 uses
//! (`ssd1306_page`: byte `page * 128 + x`, bit `y % 8`), so the display oracle,
//! `display_region` assertions and existing renderers decode it unchanged.
//! `meta.text` carries the four visible text lines as ASCII.
//!
//! Glyph shapes: the half-width cells use the public-domain X11 8x13 "fixed"
//! font (rows 2..14 of the 16-row cell), not the Sitronix HCGROM bitmap, and
//! codes 02H..1FH (symbols in the HCGROM table) are not drawn. The 16x16 CGROM
//! (BIG5/GB) is not carried at all: those cells, and the 02H..1FH symbols, are
//! counted in `meta.unknown_pixels`, which makes a region measurement over
//! them an error rather than a false "blank".
//!
//! Not modelled: cursor and blink rendering, display shift (S/C = 1 and entry
//! mode S), vertical scroll, ICON RAM, reading back in parallel modes, timing
//! (busy flag) and the parallel 8/4-bit interfaces.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

/// Glass size of a 128x64 module.
pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 64;

const DDRAM_WORDS: usize = 64;
const CGRAM_WORDS: usize = 64;
/// 64 vertical addresses x 16 horizontal words.
const GDRAM_WORDS: usize = 64 * 16;

/// Which RAM the address counter points into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Ddram,
    Cgram,
    Gdram,
    /// IRAM or vertical-scroll address (extended set): stored, not shown.
    Iram,
}

/// Serial receive state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rx {
    /// Waiting for a synchronising byte.
    Idle,
    /// Synced; the next byte carries the upper nibble.
    Upper { rs: bool, rw: bool },
    /// Upper nibble held; the next byte carries the lower nibble.
    Lower { rs: bool, rw: bool, upper: u8 },
}

pub struct St7920 {
    cs_pin: String,
    cs_active_high: bool,
    selected: bool,
    rx: Rx,
    /// Bytes that broke the framing (a nibble byte with D3..D0 set).
    framing_errors: u64,

    /// RE: extended instruction set.
    re: bool,
    /// G: graphics display on (extended function set).
    graphics_on: bool,
    /// D, C, B of display on/off control.
    display_on: bool,
    cursor_on: bool,
    blink_on: bool,
    /// I/D of entry mode set.
    increment: bool,
    /// SL = 0 entered sleep.
    sleep: bool,
    standby: bool,
    /// SR: 1 = vertical scroll address, 0 = IRAM / CGRAM address.
    sr: bool,
    /// Reverse flags of the four text lines (extended 0000_01R1R0 toggles).
    reverse: [bool; 4],

    target: Target,
    /// DDRAM / CGRAM / IRAM address counter.
    ac: u8,
    /// GDRAM vertical and horizontal address.
    gd_y: u8,
    gd_x: u8,
    /// A GDRAM address write is two instructions; true after the vertical one.
    gd_expect_x: bool,
    /// The next data byte is the low byte of the current word.
    low_byte: bool,

    ddram: [u16; DDRAM_WORDS],
    cgram: [u16; CGRAM_WORDS],
    gdram: Vec<u16>,
    component_id: Option<String>,
}

impl St7920 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            cs_active_high: true,
            selected: false,
            rx: Rx::Idle,
            framing_errors: 0,
            re: false,
            graphics_on: false,
            display_on: false,
            cursor_on: false,
            blink_on: false,
            increment: true,
            sleep: false,
            standby: false,
            sr: false,
            reverse: [false; 4],
            target: Target::Ddram,
            ac: 0,
            gd_y: 0,
            gd_x: 0,
            gd_expect_x: false,
            low_byte: false,
            // Power-on DDRAM content is undefined on silicon; spaces render
            // blank and are what a clear leaves.
            ddram: [0x2020; DDRAM_WORDS],
            cgram: [0; CGRAM_WORDS],
            gdram: vec![0; GDRAM_WORDS],
            component_id: None,
        }
    }

    /// The CS level that selects the part (HIGH per the datasheet).
    pub fn set_cs_active_high(&mut self, high: bool) {
        self.cs_active_high = high;
    }

    /// A layer is on the glass (D = 1, or G = 1 for the graphic layer) and
    /// the part is neither asleep nor in standby. G alone counts: Marlin's
    /// RepRapDiscount init leaves D = 0 and only sets G, and those panels
    /// show the picture (`marlin_rrd_init_shows_gdram_with_display_bit_clear`
    /// in `tests/display/st7920_serial.rs`).
    pub fn lit(&self) -> bool {
        (self.display_on || self.graphics_on) && !self.sleep && !self.standby
    }

    fn instruction(&mut self, v: u8) {
        // Any instruction ends standby ("any other instruction terminates").
        if !(self.re && v == 0x01) {
            self.standby = false;
        }
        // A GDRAM address is a pair of 1xxx_xxxx instructions; anything else
        // between them abandons the pair.
        let gd_pair = self.re && v & 0x80 != 0;
        if !gd_pair {
            self.gd_expect_x = false;
        }
        self.low_byte = false;
        if self.re {
            self.extended(v);
        } else {
            self.basic(v);
        }
    }

    fn basic(&mut self, v: u8) {
        match v {
            0x01 => {
                self.ddram = [0x2020; DDRAM_WORDS];
                self.target = Target::Ddram;
                self.ac = 0;
                self.increment = true;
            }
            0x02..=0x03 => {
                self.target = Target::Ddram;
                self.ac = 0;
            }
            0x04..=0x07 => self.increment = v & 0x02 != 0,
            0x08..=0x0F => {
                self.display_on = v & 0x04 != 0;
                self.cursor_on = v & 0x02 != 0;
                self.blink_on = v & 0x01 != 0;
            }
            0x10..=0x1F => {
                // S/C = 0 moves the cursor (AC); S/C = 1 shifts the display,
                // which is not modelled.
                if v & 0x08 == 0 {
                    self.target = Target::Ddram;
                    self.ac = if v & 0x04 != 0 {
                        self.ac.wrapping_add(1)
                    } else {
                        self.ac.wrapping_sub(1)
                    } & 0x3F;
                }
            }
            0x20..=0x3F => self.re = v & 0x04 != 0,
            0x40..=0x7F => {
                self.target = Target::Cgram;
                self.ac = v & 0x3F;
            }
            0x80..=0xFF => {
                // AC6 is fixed at 0.
                self.target = Target::Ddram;
                self.ac = v & 0x3F;
            }
            0x00 => {}
        }
    }

    fn extended(&mut self, v: u8) {
        match v {
            0x01 => self.standby = true,
            0x02..=0x03 => self.sr = v & 0x01 != 0,
            0x04..=0x07 => {
                let line = usize::from(v & 0x03);
                self.reverse[line] = !self.reverse[line];
            }
            0x08..=0x0F => self.sleep = v & 0x04 == 0,
            0x20..=0x3F => {
                self.re = v & 0x04 != 0;
                self.graphics_on = v & 0x02 != 0;
            }
            // 0001_xxxx is not assigned in the extended set.
            0x10..=0x1F => {}
            0x40..=0x7F => {
                self.target = Target::Iram;
                self.ac = v & 0x3F;
            }
            0x80..=0xFF => {
                if self.gd_expect_x {
                    self.gd_x = v & 0x0F;
                    self.gd_expect_x = false;
                    self.target = Target::Gdram;
                } else {
                    self.gd_y = v & 0x3F;
                    self.gd_expect_x = true;
                }
            }
            0x00 => {}
        }
    }

    /// One data byte: high byte of the current word, then low byte, then the
    /// address steps.
    fn data(&mut self, b: u8) {
        let low = self.low_byte;
        self.low_byte = !low;
        let put = |word: &mut u16| {
            *word = if low {
                (*word & 0xFF00) | u16::from(b)
            } else {
                (*word & 0x00FF) | (u16::from(b) << 8)
            };
        };
        match self.target {
            Target::Ddram => {
                put(&mut self.ddram[usize::from(self.ac & 0x3F)]);
                if low {
                    self.ac = if self.increment {
                        self.ac.wrapping_add(1)
                    } else {
                        self.ac.wrapping_sub(1)
                    } & 0x3F;
                }
            }
            Target::Cgram => {
                put(&mut self.cgram[usize::from(self.ac & 0x3F)]);
                if low {
                    self.ac = (self.ac + 1) & 0x3F;
                }
            }
            Target::Gdram => {
                let i = usize::from(self.gd_y) * 16 + usize::from(self.gd_x);
                put(&mut self.gdram[i]);
                if low {
                    self.gd_x = (self.gd_x + 1) & 0x0F;
                }
            }
            Target::Iram => {
                if low {
                    self.ac = (self.ac + 1) & 0x3F;
                }
            }
        }
    }

    fn receive(&mut self, b: u8) {
        // Five 1s, RW, RS, 0: a synchronising byte resets the serial counter.
        if b & 0xF9 == 0xF8 {
            self.rx = Rx::Upper {
                rs: b & 0x02 != 0,
                rw: b & 0x04 != 0,
            };
            return;
        }
        match self.rx {
            Rx::Idle => {}
            _ if b & 0x0F != 0 => {
                // Not a nibble byte: the frame is lost until the next sync.
                self.framing_errors += 1;
                self.rx = Rx::Idle;
            }
            Rx::Upper { rs, rw } => {
                self.rx = Rx::Lower {
                    rs,
                    rw,
                    upper: b & 0xF0,
                };
            }
            Rx::Lower { rs, rw, upper } => {
                self.rx = Rx::Upper { rs, rw };
                let v = upper | (b >> 4);
                if rw {
                    // Serial mode has no read operation.
                    return;
                }
                if rs {
                    self.data(v);
                } else {
                    self.instruction(v);
                }
            }
        }
    }

    /// The glyph pixel of the text layer at controller coordinates (`lx` in
    /// 0..256, `ly` in 0..32), or `None` when the cell's pixels are unknown.
    fn text_pixel(&self, lx: usize, ly: usize) -> Option<bool> {
        let line = ly / 16;
        let row = ly % 16;
        let word = self.ddram[line * 16 + lx / 16];
        let col = lx % 16;
        if word & 0xFFF0 == 0 {
            let glyph = usize::from((word >> 1) & 0x03);
            let bits = self.cgram[glyph * 16 + row];
            return Some(bits & (0x8000 >> col) != 0);
        }
        let hi = (word >> 8) as u8;
        if hi >= 0xA1 {
            return None;
        }
        let code = if col < 8 { hi } else { word as u8 };
        match code {
            0x20..=0x7E => {
                let glyph = &FONT_8X13[usize::from(code - 0x20)];
                let r = row.checked_sub(2).filter(|r| *r < 13);
                Some(r.is_some_and(|r| glyph[r] & (0x80 >> (col % 8)) != 0))
            }
            0x02..=0x1F | 0x7F => None,
            _ => Some(false),
        }
    }

    /// The glass as the controller drives it: `(pixels row-major, unknown)`.
    fn compose(&self) -> (Vec<bool>, usize) {
        let mut out = vec![false; WIDTH * HEIGHT];
        let mut unknown = 0;
        for py in 0..HEIGHT {
            let lx_base = if py >= 32 { 128 } else { 0 };
            let ly = py % 32;
            for px in 0..WIDTH {
                let lx = lx_base + px;
                let text = match self.text_pixel(lx, ly) {
                    Some(t) => t,
                    None => {
                        unknown += 1;
                        false
                    }
                };
                let gfx =
                    self.graphics_on && self.gdram[ly * 16 + lx / 16] & (0x8000 >> (lx % 16)) != 0;
                let rev = self.reverse[py / 16];
                out[py * WIDTH + px] = text ^ gfx ^ rev;
            }
        }
        (out, unknown)
    }

    /// The four visible text lines, top to bottom, as ASCII (non-ASCII cells
    /// are `?`, CGRAM glyphs `*`).
    pub fn text_lines(&self) -> Vec<String> {
        (0..4)
            .map(|n| {
                let base = (n % 2) * 16 + if n >= 2 { 8 } else { 0 };
                let mut s = String::with_capacity(16);
                for &word in &self.ddram[base..base + 8] {
                    if word & 0xFFF0 == 0 {
                        s.push_str("**");
                        continue;
                    }
                    for b in [(word >> 8) as u8, word as u8] {
                        s.push(match b {
                            0x20..=0x7E => b as char,
                            0x00 | 0x01 => ' ',
                            _ => '?',
                        });
                    }
                }
                s
            })
            .collect()
    }

    /// The composed picture in `ssd1306_page` packing.
    fn page_bytes(&self) -> (Vec<u8>, usize, usize) {
        let (pixels, unknown) = self.compose();
        let mut buf = vec![0u8; WIDTH * HEIGHT / 8];
        let mut lit = 0;
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                if pixels[y * WIDTH + x] {
                    buf[(y / 8) * WIDTH + x] |= 1 << (y % 8);
                    lit += 1;
                }
            }
        }
        (buf, lit, unknown)
    }
}

impl SpiDevice for St7920 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_active_high(&self) -> bool {
        self.cs_active_high
    }

    fn cs_select(&mut self) {
        self.selected = true;
        self.rx = Rx::Idle;
    }

    fn cs_release(&mut self) {
        // CS low resets the serial counter and drops a partial frame.
        self.selected = false;
        self.rx = Rx::Idle;
    }

    fn is_selected(&self) -> bool {
        self.selected
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        self.receive(mosi);
        0
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn artifacts(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        let (buf, lit_pixels, unknown) = self.page_bytes();
        let ink_bytes = buf.iter().filter(|&&b| b != 0).count();
        vec![crate::inspect::Artifact {
            kind: "framebuffer".into(),
            id: id.into(),
            meta: serde_json::json!({
                "format": crate::inspect::artifact_format::SSD1306_PAGE,
                "w": WIDTH,
                "h": HEIGHT,
                "generation": crate::inspect::artifact_generation(&buf),
                "ink_bytes": ink_bytes,
                "lit_pixels": lit_pixels,
                "unknown_pixels": unknown,
                "display_on": self.display_on,
                "lit": self.lit(),
                "graphics_on": self.graphics_on,
                "extended": self.re,
                "sleep": self.sleep,
                "cursor_on": self.cursor_on,
                "blink_on": self.blink_on,
                "text": self.text_lines(),
                "framing_errors": self.framing_errors,
            }),
            bytes: crate::inspect::artifact_bytes(&buf, opts),
        }]
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct St7920Kit;
pub static ST7920_KIT: St7920Kit = St7920Kit;

static ST7920_METADATA: KitMetadata = KitMetadata {
    inputs: std::borrow::Cow::Borrowed(&[]),
    device_type: std::borrow::Cow::Borrowed("st7920"),
    label: std::borrow::Cow::Borrowed("ST7920 128x64 LCD"),
    summary: std::borrow::Cow::Borrowed(
        "Sitronix ST7920 128x64 monochrome graphic LCD (LCD12864) in serial mode.",
    ),
    detail: std::borrow::Cow::Borrowed(
        "Serial interface framing (sync byte with RW/RS, split nibbles, CS active high), basic and \
         extended instruction sets, DDRAM text with half-width characters and four CGRAM glyphs, \
         GDRAM graphics with the 128x64 fold, display on/off, clear, home and entry mode. The \
         framebuffer artifact combines text and graphics. Half-width glyphs use a public-domain \
         8x13 font, not the Sitronix ROM bitmaps; full-width CGROM characters are reported as \
         unknown pixels. Cursor, blink, display shift, scroll and ICON RAM are not modelled.",
    ),
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: std::borrow::Cow::Borrowed(&[
        ConfigKey {
            name: std::borrow::Cow::Borrowed("cs_pin"),
            ty: ConfigType::Str,
            doc: std::borrow::Cow::Borrowed(
                "CS (RS pin in serial mode) GPIO, e.g. \"PA4\". Empty means CS tied high.",
            ),
        },
        ConfigKey {
            name: std::borrow::Cow::Borrowed("cs_active_high"),
            ty: ConfigType::Bool,
            doc: std::borrow::Cow::Borrowed(
                "CS polarity. Defaults to true (the ST7920 is selected while CS is high).",
            ),
        },
    ]),
    labs: std::borrow::Cow::Borrowed(&[]),
};

impl PeripheralKit for St7920Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &ST7920_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("").to_string();
        let mut dev = St7920::new(cs);
        dev.set_cs_active_high(ctx.config_bool("cs_active_high").unwrap_or(true));
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

/// Printable ASCII 20H..7EH, 8 pixels wide (MSB leftmost), 13 rows: the X11
/// misc-fixed 8x13 font, which is in the public domain.
#[rustfmt::skip]
const FONT_8X13: [[u8; 13]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // ' '
    [0x00, 0x00, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00, 0x10, 0x00, 0x00], // '!'
    [0x00, 0x00, 0x24, 0x24, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // '"'
    [0x00, 0x00, 0x00, 0x24, 0x24, 0x7E, 0x24, 0x7E, 0x24, 0x24, 0x00, 0x00, 0x00], // '#'
    [0x00, 0x00, 0x00, 0x10, 0x3C, 0x50, 0x38, 0x14, 0x78, 0x10, 0x00, 0x00, 0x00], // '$'
    [0x00, 0x00, 0x22, 0x52, 0x24, 0x08, 0x08, 0x10, 0x24, 0x2A, 0x44, 0x00, 0x00], // '%'
    [0x00, 0x00, 0x00, 0x00, 0x30, 0x48, 0x48, 0x30, 0x4A, 0x44, 0x3A, 0x00, 0x00], // '&'
    [0x00, 0x00, 0x38, 0x30, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // "'"
    [0x00, 0x00, 0x04, 0x08, 0x08, 0x10, 0x10, 0x10, 0x08, 0x08, 0x04, 0x00, 0x00], // '('
    [0x00, 0x00, 0x20, 0x10, 0x10, 0x08, 0x08, 0x08, 0x10, 0x10, 0x20, 0x00, 0x00], // ')'
    [0x00, 0x00, 0x00, 0x00, 0x24, 0x18, 0x7E, 0x18, 0x24, 0x00, 0x00, 0x00, 0x00], // '*'
    [0x00, 0x00, 0x00, 0x00, 0x10, 0x10, 0x7C, 0x10, 0x10, 0x00, 0x00, 0x00, 0x00], // '+'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x38, 0x30, 0x40, 0x00], // ','
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // '-'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x38, 0x10, 0x00], // '.'
    [0x00, 0x00, 0x02, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x80, 0x00, 0x00], // '/'
    [0x00, 0x00, 0x18, 0x24, 0x42, 0x42, 0x42, 0x42, 0x42, 0x24, 0x18, 0x00, 0x00], // '0'
    [0x00, 0x00, 0x10, 0x30, 0x50, 0x10, 0x10, 0x10, 0x10, 0x10, 0x7C, 0x00, 0x00], // '1'
    [0x00, 0x00, 0x3C, 0x42, 0x42, 0x02, 0x04, 0x18, 0x20, 0x40, 0x7E, 0x00, 0x00], // '2'
    [0x00, 0x00, 0x7E, 0x02, 0x04, 0x08, 0x1C, 0x02, 0x02, 0x42, 0x3C, 0x00, 0x00], // '3'
    [0x00, 0x00, 0x04, 0x0C, 0x14, 0x24, 0x44, 0x44, 0x7E, 0x04, 0x04, 0x00, 0x00], // '4'
    [0x00, 0x00, 0x7E, 0x40, 0x40, 0x5C, 0x62, 0x02, 0x02, 0x42, 0x3C, 0x00, 0x00], // '5'
    [0x00, 0x00, 0x1C, 0x20, 0x40, 0x40, 0x5C, 0x62, 0x42, 0x42, 0x3C, 0x00, 0x00], // '6'
    [0x00, 0x00, 0x7E, 0x02, 0x04, 0x08, 0x08, 0x10, 0x10, 0x20, 0x20, 0x00, 0x00], // '7'
    [0x00, 0x00, 0x3C, 0x42, 0x42, 0x42, 0x3C, 0x42, 0x42, 0x42, 0x3C, 0x00, 0x00], // '8'
    [0x00, 0x00, 0x3C, 0x42, 0x42, 0x46, 0x3A, 0x02, 0x02, 0x04, 0x38, 0x00, 0x00], // '9'
    [0x00, 0x00, 0x00, 0x00, 0x10, 0x38, 0x10, 0x00, 0x00, 0x10, 0x38, 0x10, 0x00], // ':'
    [0x00, 0x00, 0x00, 0x00, 0x10, 0x38, 0x10, 0x00, 0x00, 0x38, 0x30, 0x40, 0x00], // ';'
    [0x00, 0x00, 0x02, 0x04, 0x08, 0x10, 0x20, 0x10, 0x08, 0x04, 0x02, 0x00, 0x00], // '<'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00], // '='
    [0x00, 0x00, 0x40, 0x20, 0x10, 0x08, 0x04, 0x08, 0x10, 0x20, 0x40, 0x00, 0x00], // '>'
    [0x00, 0x00, 0x3C, 0x42, 0x42, 0x02, 0x04, 0x08, 0x08, 0x00, 0x08, 0x00, 0x00], // '?'
    [0x00, 0x00, 0x3C, 0x42, 0x42, 0x4E, 0x52, 0x56, 0x4A, 0x40, 0x3C, 0x00, 0x00], // '@'
    [0x00, 0x00, 0x18, 0x24, 0x42, 0x42, 0x42, 0x7E, 0x42, 0x42, 0x42, 0x00, 0x00], // 'A'
    [0x00, 0x00, 0xFC, 0x42, 0x42, 0x42, 0x7C, 0x42, 0x42, 0x42, 0xFC, 0x00, 0x00], // 'B'
    [0x00, 0x00, 0x3C, 0x42, 0x40, 0x40, 0x40, 0x40, 0x40, 0x42, 0x3C, 0x00, 0x00], // 'C'
    [0x00, 0x00, 0xFC, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0xFC, 0x00, 0x00], // 'D'
    [0x00, 0x00, 0x7E, 0x40, 0x40, 0x40, 0x78, 0x40, 0x40, 0x40, 0x7E, 0x00, 0x00], // 'E'
    [0x00, 0x00, 0x7E, 0x40, 0x40, 0x40, 0x78, 0x40, 0x40, 0x40, 0x40, 0x00, 0x00], // 'F'
    [0x00, 0x00, 0x3C, 0x42, 0x40, 0x40, 0x40, 0x4E, 0x42, 0x46, 0x3A, 0x00, 0x00], // 'G'
    [0x00, 0x00, 0x42, 0x42, 0x42, 0x42, 0x7E, 0x42, 0x42, 0x42, 0x42, 0x00, 0x00], // 'H'
    [0x00, 0x00, 0x7C, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x7C, 0x00, 0x00], // 'I'
    [0x00, 0x00, 0x1E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x44, 0x38, 0x00, 0x00], // 'J'
    [0x00, 0x00, 0x42, 0x44, 0x48, 0x50, 0x60, 0x50, 0x48, 0x44, 0x42, 0x00, 0x00], // 'K'
    [0x00, 0x00, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x7E, 0x00, 0x00], // 'L'
    [0x00, 0x00, 0x82, 0x82, 0xC6, 0xAA, 0x92, 0x92, 0x82, 0x82, 0x82, 0x00, 0x00], // 'M'
    [0x00, 0x00, 0x42, 0x42, 0x62, 0x52, 0x4A, 0x46, 0x42, 0x42, 0x42, 0x00, 0x00], // 'N'
    [0x00, 0x00, 0x3C, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x3C, 0x00, 0x00], // 'O'
    [0x00, 0x00, 0x7C, 0x42, 0x42, 0x42, 0x7C, 0x40, 0x40, 0x40, 0x40, 0x00, 0x00], // 'P'
    [0x00, 0x00, 0x3C, 0x42, 0x42, 0x42, 0x42, 0x42, 0x52, 0x4A, 0x3C, 0x02, 0x00], // 'Q'
    [0x00, 0x00, 0x7C, 0x42, 0x42, 0x42, 0x7C, 0x50, 0x48, 0x44, 0x42, 0x00, 0x00], // 'R'
    [0x00, 0x00, 0x3C, 0x42, 0x40, 0x40, 0x3C, 0x02, 0x02, 0x42, 0x3C, 0x00, 0x00], // 'S'
    [0x00, 0x00, 0xFE, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00, 0x00], // 'T'
    [0x00, 0x00, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x3C, 0x00, 0x00], // 'U'
    [0x00, 0x00, 0x82, 0x82, 0x44, 0x44, 0x44, 0x28, 0x28, 0x28, 0x10, 0x00, 0x00], // 'V'
    [0x00, 0x00, 0x82, 0x82, 0x82, 0x82, 0x92, 0x92, 0x92, 0xAA, 0x44, 0x00, 0x00], // 'W'
    [0x00, 0x00, 0x82, 0x82, 0x44, 0x28, 0x10, 0x28, 0x44, 0x82, 0x82, 0x00, 0x00], // 'X'
    [0x00, 0x00, 0x82, 0x82, 0x44, 0x28, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00, 0x00], // 'Y'
    [0x00, 0x00, 0x7E, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x40, 0x7E, 0x00, 0x00], // 'Z'
    [0x00, 0x00, 0x3C, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x3C, 0x00, 0x00], // '['
    [0x00, 0x00, 0x80, 0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x02, 0x00, 0x00], // backslash
    [0x00, 0x00, 0x78, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x78, 0x00, 0x00], // ']'
    [0x00, 0x00, 0x10, 0x28, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // '^'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFE, 0x00], // '_'
    [0x00, 0x00, 0x38, 0x18, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // '`'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x02, 0x3E, 0x42, 0x46, 0x3A, 0x00, 0x00], // 'a'
    [0x00, 0x00, 0x40, 0x40, 0x40, 0x5C, 0x62, 0x42, 0x42, 0x62, 0x5C, 0x00, 0x00], // 'b'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x42, 0x40, 0x40, 0x42, 0x3C, 0x00, 0x00], // 'c'
    [0x00, 0x00, 0x02, 0x02, 0x02, 0x3A, 0x46, 0x42, 0x42, 0x46, 0x3A, 0x00, 0x00], // 'd'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x42, 0x7E, 0x40, 0x42, 0x3C, 0x00, 0x00], // 'e'
    [0x00, 0x00, 0x1C, 0x22, 0x20, 0x20, 0x7C, 0x20, 0x20, 0x20, 0x20, 0x00, 0x00], // 'f'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3A, 0x44, 0x44, 0x38, 0x40, 0x3C, 0x42, 0x3C], // 'g'
    [0x00, 0x00, 0x40, 0x40, 0x40, 0x5C, 0x62, 0x42, 0x42, 0x42, 0x42, 0x00, 0x00], // 'h'
    [0x00, 0x00, 0x00, 0x10, 0x00, 0x30, 0x10, 0x10, 0x10, 0x10, 0x7C, 0x00, 0x00], // 'i'
    [0x00, 0x00, 0x00, 0x04, 0x00, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x44, 0x44, 0x38], // 'j'
    [0x00, 0x00, 0x40, 0x40, 0x40, 0x44, 0x48, 0x70, 0x48, 0x44, 0x42, 0x00, 0x00], // 'k'
    [0x00, 0x00, 0x30, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x7C, 0x00, 0x00], // 'l'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0xEC, 0x92, 0x92, 0x92, 0x92, 0x82, 0x00, 0x00], // 'm'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x5C, 0x62, 0x42, 0x42, 0x42, 0x42, 0x00, 0x00], // 'n'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x42, 0x42, 0x42, 0x42, 0x3C, 0x00, 0x00], // 'o'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x5C, 0x62, 0x42, 0x62, 0x5C, 0x40, 0x40, 0x40], // 'p'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3A, 0x46, 0x42, 0x46, 0x3A, 0x02, 0x02, 0x02], // 'q'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x5C, 0x22, 0x20, 0x20, 0x20, 0x20, 0x00, 0x00], // 'r'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3C, 0x42, 0x30, 0x0C, 0x42, 0x3C, 0x00, 0x00], // 's'
    [0x00, 0x00, 0x00, 0x20, 0x20, 0x7C, 0x20, 0x20, 0x20, 0x22, 0x1C, 0x00, 0x00], // 't'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x44, 0x44, 0x44, 0x44, 0x3A, 0x00, 0x00], // 'u'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x44, 0x44, 0x28, 0x28, 0x10, 0x00, 0x00], // 'v'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x82, 0x82, 0x92, 0x92, 0xAA, 0x44, 0x00, 0x00], // 'w'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x00, 0x00], // 'x'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x42, 0x42, 0x42, 0x46, 0x3A, 0x02, 0x42, 0x3C], // 'y'
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x04, 0x08, 0x10, 0x20, 0x7E, 0x00, 0x00], // 'z'
    [0x00, 0x00, 0x0E, 0x10, 0x10, 0x08, 0x30, 0x08, 0x10, 0x10, 0x0E, 0x00, 0x00], // '{'
    [0x00, 0x00, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x00, 0x00], // '|'
    [0x00, 0x00, 0x70, 0x08, 0x08, 0x10, 0x0C, 0x10, 0x08, 0x08, 0x70, 0x00, 0x00], // '}'
    [0x00, 0x00, 0x24, 0x54, 0x48, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // '~'
];
