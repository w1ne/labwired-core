// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Single-digit direct-drive 7-segment LED display.
//!
//! This is the bare display: eight segment pins (`A`–`G` plus `DP`) and one
//! common pin (`COM`), each wired straight to a GPIO. There is no driver chip,
//! no bus, and no protocol — the firmware simply drives the pins and the digit
//! lights up. So unlike the TM1637 (bit-banged 2-wire) or the HC595 module
//! (shifted over SPI), this model has **no state machine**: sampling is purely
//! combinational, recomputed from the GPIO output registers after every MMIO
//! write that touches a hosting port.
//!
//! The model observes the pins exactly the way
//! [`Tm1637`](super::tm1637_7seg::Tm1637) observes CLK/DIO: the
//! [`SystemBus`](crate::bus::SystemBus) re-reads the nine output bits after
//! every write to a relevant GPIO port and calls [`SevenSegment::observe_levels`].
//!
//! ## COM polarity
//!
//! The same part is sold in both wirings, and the model infers which one is in
//! use from the level the firmware holds `COM` at:
//!
//! * **COM low ⇒ common cathode.** The shared pin is the cathode tied low, so
//!   a segment conducts (lights) when its own pin is driven **HIGH**.
//! * **COM high ⇒ common anode.** The shared pin is the anode tied high, so a
//!   segment conducts (lights) when its own pin is pulled **LOW**.
//!
//! [`SevenSegment::segments`] therefore always reports *which segments are lit*,
//! in the standard `0b0gfedcba` layout (dp = bit 7), regardless of wiring —
//! callers never have to know the polarity to read the digit.

use crate::peripherals::components::seven_seg_font;

/// Segment pin order within the pin/level arrays: `A B C D E F G DP`.
/// Index `i` is bit `i` of the segment byte, so `A` = bit 0 … `G` = bit 6,
/// `DP` = bit 7 — the layout the shared font expects.
pub const SEGMENTS: usize = 8;

/// One direct-drive 7-segment digit wired to eight segment GPIOs and a common pin.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SevenSegment {
    /// board_io / external-device id.
    pub id: String,
    /// Absolute address + bit of each segment pin's GPIO **output** register
    /// (ODR), in `A B C D E F G DP` order.
    pub seg_odr: [(u64, u8); SEGMENTS],
    /// Absolute address + bit of the `COM` pin's ODR.
    pub com_odr_addr: u64,
    pub com_bit: u8,
    /// Cached peripheral indices of the GPIO ports hosting each pin, resolved
    /// lazily on first use by the bus write-hook. `None` until resolved.
    #[serde(skip)]
    seg_peripheral_idx: [Option<usize>; SEGMENTS],
    #[serde(skip)]
    com_peripheral_idx: Option<usize>,

    /// Currently **lit** segments, `0b0gfedcba` with dp on bit 7. Polarity has
    /// already been folded in, so this is what a human sees.
    lit: u8,
}

impl SevenSegment {
    pub fn new(
        id: impl Into<String>,
        seg_odr: [(u64, u8); SEGMENTS],
        com_odr_addr: u64,
        com_bit: u8,
    ) -> Self {
        Self {
            id: id.into(),
            seg_odr,
            com_odr_addr,
            com_bit,
            seg_peripheral_idx: [None; SEGMENTS],
            com_peripheral_idx: None,
            lit: 0,
        }
    }

    /// Recompute the lit-segment mask from the current pin levels.
    ///
    /// `seg_levels[i]` is the driven level of segment pin `i` (`A`…`DP`), and
    /// `com` is the level of the common pin. Combinational: the result depends
    /// only on this call's levels, never on history.
    pub fn observe_levels(&mut self, seg_levels: [bool; SEGMENTS], com: bool) {
        // com == false → common cathode → segment lit when its pin is HIGH.
        // com == true  → common anode   → segment lit when its pin is LOW.
        let mut mask = 0u8;
        for (i, level) in seg_levels.iter().enumerate() {
            let lit = if com { !*level } else { *level };
            if lit {
                mask |= 1 << i;
            }
        }
        self.lit = mask;
    }

    /// The lit segments as `0b0gfedcba` (dp = bit 7). Polarity-normalised.
    pub fn segments(&self) -> u8 {
        self.lit
    }

    /// The character currently displayed, decoded through the shared segment
    /// font. Blank is `' '`; an unrecognised pattern is `'?'`.
    pub fn ch(&self) -> char {
        seven_seg_font::decode(self.lit)
    }

    /// True when the decimal-point segment is lit.
    pub fn decimal_point(&self) -> bool {
        self.lit & 0x80 != 0
    }

    // ─── Cached GPIO peripheral indices (used by the bus write-hook) ───

    pub(crate) fn seg_peripheral_idx(&self, i: usize) -> Option<usize> {
        self.seg_peripheral_idx[i]
    }
    pub(crate) fn set_seg_peripheral_idx(&mut self, i: usize, idx: usize) {
        self.seg_peripheral_idx[i] = Some(idx);
    }
    pub(crate) fn com_peripheral_idx(&self) -> Option<usize> {
        self.com_peripheral_idx
    }
    pub(crate) fn set_com_peripheral_idx(&mut self, idx: usize) {
        self.com_peripheral_idx = Some(idx);
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct SevenSegmentKit;
pub static SEVEN_SEGMENT_KIT: SevenSegmentKit = SevenSegmentKit;

/// Config key names for the eight segment pins, in `A`…`DP` order, paired with
/// the default pin each falls back to.
const SEG_KEYS: [(&str, &str); SEGMENTS] = [
    ("a_pin", "PA0"),
    ("b_pin", "PA1"),
    ("c_pin", "PA2"),
    ("d_pin", "PA3"),
    ("e_pin", "PA4"),
    ("f_pin", "PA5"),
    ("g_pin", "PA6"),
    ("dp_pin", "PA7"),
];
const COM_KEY: (&str, &str) = ("com_pin", "PA8");

static SEVEN_SEGMENT_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "seven-segment",
    label: "7-Segment Display (single digit)",
    summary: "Bare single-digit 7-segment LED display driven straight from nine GPIO pins.",
    detail: "The raw display with no driver chip: segment pins A–G plus DP and one common pin, \
             each wired directly to a GPIO. The model samples all nine output registers after \
             every write to a hosting port and recomputes the lit segments combinationally — \
             there is no protocol or state machine. COM polarity is inferred from the level the \
             firmware holds the common pin at: COM low means common cathode (a segment lights \
             when its pin is HIGH), COM high means common anode (a segment lights when its pin \
             is LOW). The readback always reports which segments are lit regardless of wiring, \
             decoded to a character through the standard a–g/dp segment font.",
    transport: Transport::GpioGroup,
    category: Category::Gpio,
    config_keys: &[
        ConfigKey {
            name: "a_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving segment A (e.g. \"PA0\"). Defaults to PA0.",
        },
        ConfigKey {
            name: "b_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving segment B (e.g. \"PA1\"). Defaults to PA1.",
        },
        ConfigKey {
            name: "c_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving segment C (e.g. \"PA2\"). Defaults to PA2.",
        },
        ConfigKey {
            name: "d_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving segment D (e.g. \"PA3\"). Defaults to PA3.",
        },
        ConfigKey {
            name: "e_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving segment E (e.g. \"PA4\"). Defaults to PA4.",
        },
        ConfigKey {
            name: "f_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving segment F (e.g. \"PA5\"). Defaults to PA5.",
        },
        ConfigKey {
            name: "g_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving segment G (e.g. \"PA6\"). Defaults to PA6.",
        },
        ConfigKey {
            name: "dp_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving the decimal point (e.g. \"PA7\"). Defaults to PA7.",
        },
        ConfigKey {
            name: "com_pin",
            ty: ConfigType::Str,
            doc: "GPIO pin driving the common pin (e.g. \"PA8\"). Its level selects the wiring: \
                  low = common cathode, high = common anode. Defaults to PA8.",
        },
    ],
    // No lab yet: shipping a LabRef would promise a one-click demo that 404s.
    labs: &[],
};

impl PeripheralKit for SevenSegmentKit {
    fn metadata(&self) -> &'static KitMetadata {
        &SEVEN_SEGMENT_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let mut seg_odr = [(0u64, 0u8); SEGMENTS];
        for (i, (key, default)) in SEG_KEYS.iter().enumerate() {
            let pin = ctx.config_str(key).unwrap_or(default).to_string();
            seg_odr[i] = ctx.resolve_pin_odr(&pin).ok_or_else(|| {
                anyhow::anyhow!(
                    "7-segment '{}' {} '{}' could not be resolved to a GPIO",
                    ctx.device_id(),
                    key,
                    pin
                )
            })?;
        }
        let com = ctx.config_str(COM_KEY.0).unwrap_or(COM_KEY.1).to_string();
        let (com_addr, com_bit) = ctx.resolve_pin_odr(&com).ok_or_else(|| {
            anyhow::anyhow!(
                "7-segment '{}' {} '{}' could not be resolved to a GPIO",
                ctx.device_id(),
                COM_KEY.0,
                com
            )
        })?;
        let id = ctx.device_id().to_string();
        ctx.bus
            .seven_segment
            .push(SevenSegment::new(id, seg_odr, com_addr, com_bit));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin wiring is irrelevant to the combinational tests — any addresses do.
    fn new_dev() -> SevenSegment {
        let seg = std::array::from_fn(|i| (0x4002_0014u64, i as u8));
        SevenSegment::new("seg", seg, 0x4002_0014, 8)
    }

    /// Segment levels from a `0b0gfedcba` byte (bit i → pin i).
    fn levels(byte: u8) -> [bool; SEGMENTS] {
        std::array::from_fn(|i| (byte >> i) & 1 != 0)
    }

    #[test]
    fn common_cathode_lights_segments_driven_high() {
        let mut dev = new_dev();
        // A and B high, COM low → exactly A+B lit.
        dev.observe_levels(levels(0b0000_0011), false);
        assert_eq!(dev.segments(), 0b0000_0011);
    }

    #[test]
    fn common_anode_lights_segments_driven_low() {
        let mut dev = new_dev();
        // Same two segments, inverted drive, COM high → still exactly A+B lit.
        dev.observe_levels(levels(!0b0000_0011), true);
        assert_eq!(dev.segments(), 0b0000_0011);
    }

    #[test]
    fn digit_patterns_decode_through_shared_font() {
        let mut dev = new_dev();
        dev.observe_levels(levels(0x3F), false);
        assert_eq!(dev.ch(), '0');
        dev.observe_levels(levels(0x7F), false);
        assert_eq!(dev.ch(), '8');
    }

    #[test]
    fn decimal_point_bit_is_reported_and_excluded_from_the_glyph() {
        let mut dev = new_dev();
        dev.observe_levels(levels(0x3F | 0x80), false);
        assert!(dev.decimal_point());
        assert_eq!(dev.ch(), '0');

        dev.observe_levels(levels(0x3F), false);
        assert!(!dev.decimal_point());
    }

    #[test]
    fn all_segments_off_reads_as_blank() {
        let mut dev = new_dev();
        dev.observe_levels(levels(0x00), false);
        assert_eq!(dev.segments(), 0);
        assert_eq!(dev.ch(), ' ');
        assert!(!dev.decimal_point());

        // Common anode blank = every pin held high.
        dev.observe_levels(levels(0xFF), true);
        assert_eq!(dev.segments(), 0);
        assert_eq!(dev.ch(), ' ');
    }

    #[test]
    fn sampling_is_combinational_across_calls() {
        let mut dev = new_dev();
        dev.observe_levels(levels(0x3F), false); // '0'
        assert_eq!(dev.ch(), '0');
        // Add segment G → '8'. No latching, no history: the new levels win.
        dev.observe_levels(levels(0x7F), false);
        assert_eq!(dev.ch(), '8');
        // Drop back to '1'.
        dev.observe_levels(levels(0x06), false);
        assert_eq!(dev.ch(), '1');
    }

    /// End-to-end through the bus write-hook: drive the segment pins via BSRR
    /// the way firmware does and read the digit back off the bus vector.
    #[test]
    fn driven_through_bus_write_hook() {
        use crate::bus::SystemBus;
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::Bus;

        const GPIOA: u64 = 0x4800_0000; // stm32v2: ODR @ 0x14, BSRR @ 0x18
        const ODR: u64 = GPIOA + 0x14;
        const BSRR: u64 = GPIOA + 0x18;

        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "gpioa",
            GPIOA,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        // Segments A..DP on bits 0..7, COM on bit 8 — all one port.
        let seg = std::array::from_fn(|i| (ODR, i as u8));
        bus.seven_segment
            .push(SevenSegment::new("seg", seg, ODR, 8));

        // Common cathode: hold COM low, drive the '0' pattern high.
        let set = |bus: &mut SystemBus, segs: u8, com: bool| {
            let mut v = 0u32;
            for i in 0..8u8 {
                v |= if (segs >> i) & 1 != 0 {
                    1u32 << i
                } else {
                    1u32 << (i + 16)
                };
            }
            v |= if com { 1u32 << 8 } else { 1u32 << (8 + 16) };
            Bus::write_u32(bus, BSRR, v).unwrap();
        };

        set(&mut bus, 0x3F, false);
        assert_eq!(bus.seven_segment[0].ch(), '0');
        assert_eq!(bus.seven_segment[0].segments(), 0x3F);

        // Changing the pins updates the readback on the next write.
        set(&mut bus, 0x06, false);
        assert_eq!(bus.seven_segment[0].ch(), '1');

        // Flip to common anode: COM high, pins inverted → same glyph.
        set(&mut bus, !0x06, true);
        assert_eq!(bus.seven_segment[0].ch(), '1');
    }
}
