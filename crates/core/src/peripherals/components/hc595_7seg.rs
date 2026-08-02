// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! 4-digit 7-segment LED display driven by two chained 74HC595 shift registers.
//!
//! This models the ubiquitous blue "4-bit LED display module" (e.g. the QIFEI
//! board carrying two `74HC595` in series). The host drives it exactly like an
//! SPI slave: `DIO`→MOSI, `SCLK`→SCK, `RCLK`→chip-select/latch. For each digit
//! the firmware shifts **16 bits** — one segment byte and one digit-select byte
//! — then pulses `RCLK` to latch, and cycles through the four digits fast enough
//! that persistence of vision fuses them into a steady 4-character readout.
//!
//! Because the two registers are chained, the model does not care which physical
//! byte is which: it auto-detects the digit-select byte as the one that is
//! one-hot across the four common lines (active-high `0x1/0x2/0x4/0x8` or the
//! active-low complement) and treats the other byte as segments. Each latched
//! frame updates that digit's segment pattern; [`Hc5957Seg::text`] renders the
//! four decoded characters.

use crate::peripherals::components::seven_seg_font;
use crate::peripherals::spi::SpiDevice;
use std::any::Any;

const DIGITS: usize = 4;

/// Simulated dual-74HC595 4-digit 7-segment LED display.
#[derive(Debug, serde::Serialize)]
pub struct Hc5957Seg {
    /// Latch line (`RCLK`), wired to the GPIO used as SPI chip-select.
    cs_pin: String,
    /// Two-byte shift accumulator for the frame currently being clocked in.
    shift: [u8; 2],
    /// Number of bytes clocked into `shift` since the last latch.
    shift_len: u8,
    /// Latched segment byte per digit (index 0 = leftmost digit).
    segments: [u8; DIGITS],
}

impl Hc5957Seg {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            shift: [0; 2],
            shift_len: 0,
            segments: [0; DIGITS],
        }
    }

    /// Latched raw segment byte for `digit` (0..4), `0b0gfedcba` (dp = bit 7).
    pub fn segment_byte(&self, digit: usize) -> u8 {
        self.segments.get(digit).copied().unwrap_or(0)
    }

    /// Decode a latched segment byte to the character it displays. The decimal
    /// point (bit 7) is ignored for the glyph match; unknown patterns render as
    /// `?` so a mis-driven panel is visible rather than silently blank.
    fn decode(seg: u8) -> char {
        seven_seg_font::decode(seg)
    }

    /// The four decoded characters, leftmost digit first.
    pub fn chars(&self) -> [char; DIGITS] {
        let mut out = [' '; DIGITS];
        for (i, seg) in self.segments.iter().enumerate() {
            out[i] = Self::decode(*seg);
        }
        out
    }

    /// True when digit `i` has its decimal-point segment (bit 7) lit.
    pub fn decimal_point(&self, digit: usize) -> bool {
        self.segments.get(digit).is_some_and(|s| s & 0x80 != 0)
    }

    /// The whole panel as a 4-char string (for logs / assertions / the bridge).
    pub fn text(&self) -> String {
        self.chars().iter().collect()
    }

    /// Return the one-hot digit index encoded by `byte`, if it selects exactly
    /// one of the four common lines — active-high (`0x1/0x2/0x4/0x8`) or the
    /// active-low complement. Returns `None` when the byte is not a valid
    /// one-hot digit select (so the caller knows it must be the segment byte).
    fn digit_select_index(byte: u8) -> Option<usize> {
        // Active-high select is one of 0x01/0x02/0x04/0x08; active-low is the
        // bitwise complement (0xFE/0xFD/0xFB/0xF7). In both cases exactly one of
        // the four low bits is the "selected" line and the high nibble carries
        // no digit lines. No standard 7-segment glyph is a single low-nibble
        // bit, so this never mistakes segment data for a digit select.
        for candidate in [byte, !byte] {
            if candidate & 0xF0 == 0 && (candidate & 0x0F).count_ones() == 1 {
                return Some((candidate & 0x0F).trailing_zeros() as usize);
            }
        }
        None
    }

    /// Process a fully clocked-in 16-bit frame: identify which byte is the
    /// digit select and which is the segments, then latch the segments into
    /// that digit.
    fn latch_frame(&mut self) {
        let (b0, b1) = (self.shift[0], self.shift[1]);
        // Try each byte as the digit-select; the other is the segment data.
        let resolved = Self::digit_select_index(b1)
            .map(|d| (d, b0))
            .or_else(|| Self::digit_select_index(b0).map(|d| (d, b1)));
        if let Some((digit, seg)) = resolved {
            if digit < DIGITS {
                self.segments[digit] = seg;
            }
        }
    }

    fn push_byte(&mut self, byte: u8) {
        let idx = (self.shift_len as usize) % 2;
        self.shift[idx] = byte;
        self.shift_len += 1;
        if self.shift_len == 2 {
            self.latch_frame();
            self.shift_len = 0;
        }
    }
}

impl SpiDevice for Hc5957Seg {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        // RCLK asserted → start of a fresh 16-bit shift. Discard any partial
        // frame so a byte-misaligned burst can't smear across digits.
        self.shift_len = 0;
    }

    fn cs_release(&mut self) {
        // A frame is latched every two bytes in `push_byte`; nothing extra to
        // do on release. Reset the partial counter for the next assertion.
        self.shift_len = 0;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        self.push_byte(mosi);
        0 // shift registers have no meaningful MISO on this module.
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Hc5957SegKit;
pub static HC595_7SEG_KIT: Hc5957SegKit = Hc5957SegKit;

static HC595_7SEG_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "hc595-7seg",
    label: "74HC595 7-Segment (4-digit)",
    summary:
        "4-digit 7-segment LED display driven by two chained 74HC595 shift registers over SPI.",
    detail: "The classic blue 4-digit module: a segment 74HC595 and a digit-select 74HC595 in \
             series. Firmware shifts a segment byte then a digit-select byte (16 bits), pulses \
             RCLK (SPI chip-select) to latch, and multiplexes the four digits. The model \
             auto-detects the digit-select byte, decodes the standard a–g/dp segment font, and \
             surfaces the four displayed characters.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "RCLK latch GPIO pin, wired as SPI chip-select (e.g. \"PA4\"). Defaults to PA4.",
    }],
    // No lab yet: examples/hc595-7seg-lab has only a README + system.yaml — no demo
    // firmware/ELF is built or published. Declaring a LabRef would promise a
    // one-click demo that 404s (the playground gate rightly rejects it).
    // Re-add the LabRef when the demo firmware ships.
    labs: &[],
};

impl PeripheralKit for Hc5957SegKit {
    fn metadata(&self) -> &'static KitMetadata {
        &HC595_7SEG_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs_pin = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        ctx.attach_spi_device(Box::new(Hc5957Seg::new(cs_pin)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::spi::SpiDevice;

    /// Shift `segments` then a one-hot active-high `digit` select, the way the
    /// module's driver multiplexes one digit.
    fn write_digit(dev: &mut Hc5957Seg, segments: u8, digit: usize) {
        dev.cs_select();
        dev.transfer(segments);
        dev.transfer(1u8 << digit);
        dev.cs_release();
    }

    #[test]
    fn decodes_multiplexed_digits_into_text() {
        let mut dev = Hc5957Seg::new("PA4");
        // "1234" across the four digits (font bytes for 1,2,3,4).
        write_digit(&mut dev, 0x06, 0); // '1'
        write_digit(&mut dev, 0x5B, 1); // '2'
        write_digit(&mut dev, 0x4F, 2); // '3'
        write_digit(&mut dev, 0x66, 3); // '4'
        assert_eq!(dev.text(), "1234");
    }

    #[test]
    fn auto_detects_digit_select_regardless_of_byte_order() {
        let mut dev = Hc5957Seg::new("PA4");
        // Send the digit-select byte FIRST, segments second (reversed chain).
        dev.cs_select();
        dev.transfer(1u8 << 2); // digit 2 select
        dev.transfer(0x3F); // '0'
        dev.cs_release();
        assert_eq!(dev.chars()[2], '0');
    }

    #[test]
    fn tracks_decimal_point_bit() {
        let mut dev = Hc5957Seg::new("PA4");
        write_digit(&mut dev, 0x3F | 0x80, 1); // '0' with dp
        assert_eq!(dev.chars()[1], '0');
        assert!(dev.decimal_point(1));
        assert!(!dev.decimal_point(0));
    }

    #[test]
    fn unknown_pattern_renders_as_question_mark() {
        let mut dev = Hc5957Seg::new("PA4");
        write_digit(&mut dev, 0x2A, 0); // not a valid glyph
        assert_eq!(dev.chars()[0], '?');
    }
}
