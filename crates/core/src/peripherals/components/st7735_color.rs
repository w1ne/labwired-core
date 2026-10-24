// SPDX-License-Identifier: MIT

//! ST7735R serial colour-decoding foundation, NOT a connected display model.
//!
//! Register/bit facts: ST7735R v0.2, sections 9.8.20–22, 9.15, 9.18 and
//! 10.1.22: <https://cdn-shop.adafruit.com/datasheets/ST7735R_V0.2.pdf>.
//! The output is the controller's six-bit-per-channel memory value, not an
//! RGB888 rendering or a physical glass coordinate. MADCTL/BGR, address
//! windows, CS/DC command parsing, reset GPIO and backlight belong to the
//! future integration. No shipping descriptor uses this module yet.

/// A complete pixel consumes a counter step even if its colour is undefined.
/// Never substitute black or a guessed linear LUT for `UndefinedLookup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodedPixel {
    Rgb666([u8; 3]),
    UndefinedLookup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerialFormat {
    Rgb444,
    Rgb565,
    Rgb666,
}

impl SerialFormat {
    /// COLMOD's low three bits; parallel-interface bits are not serial depth.
    /// Reserved depths are unsupported, not silently interpreted as RGB565.
    pub fn from_colmod(value: u8) -> Option<Self> {
        match value & 7 {
            3 => Some(Self::Rgb444),
            5 => Some(Self::Rgb565),
            6 => Some(Self::Rgb666),
            _ => None,
        }
    }
}

/// Pure byte codec. A caller supplies ONLY RAMWR data bytes, and explicitly
/// discards partial pixels at a command/stream boundary. Ordinary delivery
/// chunks do not reset it. This API makes no assertion about CS edge framing.
#[derive(Clone, Debug)]
pub struct St7735ColorDecoder {
    format: SerialFormat,
    lut: Option<[u8; 128]>,
    bytes: [u8; 3],
    have: usize,
}

impl Default for St7735ColorDecoder {
    fn default() -> Self {
        Self {
            format: SerialFormat::Rgb666,
            // Hardware/power-on LUT values are unspecified, not a ramp.
            lut: None,
            bytes: [0; 3],
            have: 0,
        }
    }
}

impl St7735ColorDecoder {
    pub fn format(&self) -> SerialFormat {
        self.format
    }

    /// Integration policy: reject unsupported serial depths without mutation.
    /// This is not a claim about silicon's handling of reserved COLMOD values.
    pub fn set_colmod(&mut self, value: u8) -> bool {
        let Some(format) = SerialFormat::from_colmod(value) else {
            return false;
        };
        self.discard_partial();
        self.format = format;
        true
    }

    /// A COMPLETE RGBSET payload, in R32/G64/B32 order. Only D5..D0 matter.
    /// No RAM is owned here: installing a LUT cannot recolour prior pixels.
    /// Partial/aborted RGBSET wire semantics are intentionally not implemented.
    pub fn install_complete_lut(&mut self, bytes: &[u8; 128]) {
        self.lut = Some(bytes.map(|byte| byte & 0x3f));
    }

    /// A parser must not retain a previous valid table after an incomplete
    /// RGBSET. Call when an upload begins; restore validity only on completion.
    /// Exact partially written silicon table contents remain unmodelled.
    pub fn invalidate_lut(&mut self) {
        self.lut = None;
    }

    pub fn discard_partial(&mut self) {
        self.have = 0;
        self.bytes = [0; 3];
    }

    /// Software reset retains colour depth and LUT, and owns no frame RAM.
    pub fn software_reset(&mut self) {
        self.discard_partial();
    }

    /// Hardware reset restores RGB666 and invalidates the unspecified LUT.
    /// A future panel integration must retain RAM rather than clearing it.
    pub fn hardware_reset(&mut self) {
        *self = Self::default();
    }

    fn lookup(&self, r: usize, g: usize, b: usize) -> DecodedPixel {
        match &self.lut {
            Some(lut) => DecodedPixel::Rgb666([lut[r], lut[32 + g], lut[96 + b]]),
            None => DecodedPixel::UndefinedLookup,
        }
    }

    /// Zero or one completed pixel for one byte. RGB444 emits its first pixel
    /// after byte two and its second after byte three: R1G1 / B1R2 / G2B2.
    /// An undefined lookup still returns `Some`, preserving pixel consumption.
    pub fn push(&mut self, byte: u8) -> Option<DecodedPixel> {
        self.bytes[self.have] = byte;
        self.have += 1;
        match self.format {
            SerialFormat::Rgb444 => match self.have {
                2 => Some(self.lookup(
                    (self.bytes[0] >> 4) as usize,
                    (self.bytes[0] & 15) as usize,
                    (byte >> 4) as usize,
                )),
                3 => {
                    self.have = 0;
                    Some(self.lookup(
                        (self.bytes[1] & 15) as usize,
                        (byte >> 4) as usize,
                        (byte & 15) as usize,
                    ))
                }
                _ => None,
            },
            SerialFormat::Rgb565 if self.have == 2 => {
                self.have = 0;
                let bits = u16::from_be_bytes([self.bytes[0], byte]);
                Some(self.lookup(
                    (bits >> 11) as usize,
                    ((bits >> 5) & 63) as usize,
                    (bits & 31) as usize,
                ))
            }
            SerialFormat::Rgb666 if self.have == 3 => {
                self.have = 0;
                Some(DecodedPixel::Rgb666(self.bytes.map(|value| value >> 2)))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lut() -> [u8; 128] {
        // Deliberately non-linear and channel-distinct; a scale/shift decoder
        // or a G/B segment offset mistake must not pass these expectations.
        std::array::from_fn(|i| ((i * 19 + i / 32 * 7 + 11) & 63) as u8)
    }

    fn ready(colmod: u8) -> St7735ColorDecoder {
        let mut decoder = St7735ColorDecoder::default();
        assert!(decoder.set_colmod(colmod));
        decoder.install_complete_lut(&lut());
        decoder
    }

    #[test]
    fn st7735_color_hardware_default_and_rgb666_ignore_low_bits_and_lut() {
        let mut decoder = St7735ColorDecoder::default();
        assert_eq!(decoder.format(), SerialFormat::Rgb666);
        for value in 0..64u8 {
            for low in 0..4 {
                assert_eq!(decoder.push(value << 2 | low), None);
                assert_eq!(decoder.push((63 - value) << 2 | low), None);
                assert_eq!(
                    decoder.push(17 << 2 | low),
                    Some(DecodedPixel::Rgb666([value, 63 - value, 17]))
                );
            }
        }
        decoder.install_complete_lut(&[0; 128]);
        assert_eq!(decoder.push(0xfc), None);
        assert_eq!(decoder.push(0x80), None);
        assert_eq!(decoder.push(0x04), Some(DecodedPixel::Rgb666([63, 32, 1])));
    }

    #[test]
    fn st7735_color_packed_rgb444_distinct_pixels_and_continuous_groups() {
        let mut decoder = ready(3);
        let table = lut();
        for _ in 0..3 {
            assert_eq!(decoder.push(0x12), None);
            assert_eq!(
                decoder.push(0x34),
                Some(DecodedPixel::Rgb666([table[1], table[34], table[99]]))
            );
            assert_eq!(
                decoder.push(0x56),
                Some(DecodedPixel::Rgb666([table[4], table[37], table[102]]))
            );
        }
    }

    #[test]
    fn st7735_color_all_rgb444_values_use_first_sixteen_entries_per_channel() {
        let mut decoder = ready(3);
        let table = lut();
        for bits in 0..4096u16 {
            let r = usize::from(bits >> 8);
            let g = usize::from((bits >> 4) & 15);
            let b = usize::from(bits & 15);
            let expected = Some(DecodedPixel::Rgb666([
                table[r],
                table[32 + g],
                table[96 + b],
            ]));
            assert_eq!(decoder.push((bits >> 4) as u8), None);
            assert_eq!(decoder.push(((bits & 15) << 4 | bits >> 8) as u8), expected);
            assert_eq!(decoder.push(bits as u8), expected);
        }
    }

    #[test]
    fn st7735_color_all_rgb565_values_use_full_lut_segments() {
        let mut decoder = ready(5);
        let table = lut();
        for bits in 0..=u16::MAX {
            let [high, low] = bits.to_be_bytes();
            assert_eq!(decoder.push(high), None);
            assert_eq!(
                decoder.push(low),
                Some(DecodedPixel::Rgb666([
                    table[usize::from(bits >> 11)],
                    table[32 + usize::from((bits >> 5) & 63)],
                    table[96 + usize::from(bits & 31)],
                ]))
            );
        }
    }

    #[test]
    fn st7735_color_unknown_lut_consumes_pixels_without_inventing_colours() {
        let mut decoder = St7735ColorDecoder::default();
        assert!(decoder.set_colmod(3));
        assert_eq!(decoder.push(0x12), None);
        assert_eq!(decoder.push(0x34), Some(DecodedPixel::UndefinedLookup));
        assert_eq!(decoder.push(0x56), Some(DecodedPixel::UndefinedLookup));
        assert!(decoder.set_colmod(5));
        assert_eq!(decoder.push(0x00), None);
        assert_eq!(decoder.push(0x00), Some(DecodedPixel::UndefinedLookup));
    }

    #[test]
    fn st7735_color_complete_lut_masks_reserved_bits_and_affects_future_only() {
        let mut decoder = ready(5);
        assert_eq!(decoder.push(0x00), None);
        let old_pixel = decoder.push(0x00);
        decoder.install_complete_lut(&[0xff; 128]);
        assert_eq!(decoder.push(0x00), None);
        assert_eq!(decoder.push(0x00), Some(DecodedPixel::Rgb666([63; 3])));
        let table = lut();
        assert_eq!(
            old_pixel,
            Some(DecodedPixel::Rgb666([table[0], table[32], table[96]]))
        );
    }

    #[test]
    fn st7735_color_aborted_lut_upload_cannot_reuse_previous_valid_table() {
        let mut decoder = ready(5);
        decoder.invalidate_lut();
        decoder.software_reset();
        assert_eq!(decoder.push(0x00), None);
        assert_eq!(decoder.push(0x00), Some(DecodedPixel::UndefinedLookup));
        decoder.install_complete_lut(&[27; 128]);
        assert_eq!(decoder.push(0x00), None);
        assert_eq!(decoder.push(0x00), Some(DecodedPixel::Rgb666([27; 3])));
    }

    #[test]
    fn st7735_color_delivery_chunks_do_not_change_pixel_sequence() {
        let bytes = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];
        for depth in [3, 5, 6] {
            let mut reference = ready(depth);
            let expected: Vec<_> = bytes
                .iter()
                .filter_map(|&byte| reference.push(byte))
                .collect();
            // All possible chunk splits, including between both halves of the
            // RGB444 middle byte's pixels. No stream boundary is implied.
            for split_mask in 0..32 {
                let mut decoder = ready(depth);
                let mut actual = Vec::new();
                let mut start = 0;
                for end in 1..=bytes.len() {
                    if end == bytes.len() || split_mask & (1 << (end - 1)) != 0 {
                        actual.extend(bytes[start..end].iter().filter_map(|&b| decoder.push(b)));
                        start = end;
                    }
                }
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn st7735_color_discard_every_partial_position_and_restart() {
        for (depth, group) in [(3, 3), (5, 2), (6, 3)] {
            for prefix in 0..group {
                let mut decoder = ready(depth);
                for _ in 0..prefix {
                    let _ = decoder.push(0xff);
                }
                decoder.discard_partial();
                let mut fresh = ready(depth);
                for byte in [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc] {
                    assert_eq!(decoder.push(byte), fresh.push(byte));
                }
            }
        }
    }

    #[test]
    fn st7735_color_software_reset_retains_format_and_lut_hardware_does_not() {
        let mut decoder = ready(3);
        assert_eq!(decoder.push(0xff), None);
        decoder.software_reset();
        assert_eq!(decoder.format(), SerialFormat::Rgb444);
        assert_eq!(decoder.push(0x12), None);
        let table = lut();
        assert_eq!(
            decoder.push(0x34),
            Some(DecodedPixel::Rgb666([table[1], table[34], table[99]]))
        );
        decoder.hardware_reset();
        assert_eq!(decoder.format(), SerialFormat::Rgb666);
        assert!(decoder.set_colmod(5));
        assert_eq!(decoder.push(0x12), None);
        assert_eq!(decoder.push(0x34), Some(DecodedPixel::UndefinedLookup));
    }

    #[test]
    fn st7735_color_colmod_change_discards_partial_reserved_depth_rejects_unchanged() {
        let mut decoder = ready(3);
        assert_eq!(decoder.push(0xff), None);
        assert!(decoder.set_colmod(0x65));
        assert_eq!(decoder.format(), SerialFormat::Rgb565);
        assert_eq!(decoder.push(0x12), None);
        for value in [0, 1, 2, 4, 7, 0xff] {
            assert!(!decoder.set_colmod(value));
            assert_eq!(decoder.format(), SerialFormat::Rgb565);
        }
        let table = lut();
        assert_eq!(
            decoder.push(0x34),
            Some(DecodedPixel::Rgb666([table[2], table[49], table[116]]))
        );
    }
}
