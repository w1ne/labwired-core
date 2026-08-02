// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The standard 7-segment character font, shared by every module that renders
//! a segment byte back to the character a human reads off the panel.
//!
//! Bit layout is `0b0gfedcba` — segment `a` = bit 0 … segment `g` = bit 6 —
//! with the decimal point on bit 7. This is the near-universal ordering used by
//! the 74HC595 display modules, the TM1637, and bare direct-drive digits, so
//! one table serves them all: forking it per driver is how the renderings
//! silently drift apart.

/// Segment-pattern → character table, `0b0gfedcba` (dp excluded).
pub const FONT: &[(u8, char)] = &[
    (0x3F, '0'),
    (0x06, '1'),
    (0x5B, '2'),
    (0x4F, '3'),
    (0x66, '4'),
    (0x6D, '5'),
    (0x7D, '6'),
    (0x07, '7'),
    (0x7F, '8'),
    (0x6F, '9'),
    (0x77, 'A'),
    (0x7C, 'b'),
    (0x39, 'C'),
    (0x5E, 'd'),
    (0x79, 'E'),
    (0x71, 'F'),
    (0x40, '-'),
    (0x00, ' '),
];

/// Decode a segment byte to the character it displays.
///
/// The decimal point (bit 7) is masked off before the glyph match — callers
/// that care about it read the bit themselves. Unknown patterns render as `?`
/// so a mis-driven panel is visible rather than silently blank.
pub fn decode(seg: u8) -> char {
    let glyph = seg & 0x7F;
    FONT.iter()
        .find(|(pattern, _)| *pattern == glyph)
        .map(|(_, ch)| *ch)
        .unwrap_or('?')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_digits_and_hex_letters() {
        assert_eq!(decode(0x3F), '0');
        assert_eq!(decode(0x7F), '8');
        assert_eq!(decode(0x77), 'A');
        assert_eq!(decode(0x71), 'F');
    }

    #[test]
    fn blank_pattern_is_space_and_dash_is_g_only() {
        assert_eq!(decode(0x00), ' ');
        assert_eq!(decode(0x40), '-');
    }

    #[test]
    fn decimal_point_is_masked_off_before_matching() {
        // dp set must not change the glyph.
        assert_eq!(decode(0x3F | 0x80), '0');
        // Blank glyph + dp only.
        assert_eq!(decode(0x80), ' ');
    }

    #[test]
    fn unknown_pattern_renders_as_question_mark() {
        assert_eq!(decode(0x01 | 0x08), '?');
    }
}
