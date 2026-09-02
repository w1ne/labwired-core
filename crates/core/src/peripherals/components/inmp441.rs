// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! TDK InvenSense INMP441 omnidirectional MEMS microphone, I2S digital output.
//!
//! Facts below are from the INMP441 datasheet (the document HEStore serves for
//! article 100.431.82, 22pp, sha256 4674fcfe...):
//!
//! * Pin 4 L/R, p.9: "When set low, the microphone outputs its signal in the
//!   left channel of the I2S frame. When set high, ... the right channel."
//! * §I2S DATA INTERFACE, p.14: "The slave serial-data port's format is I2S,
//!   24-bit, twos complement. There must be 64 SCK cycles in each WS stereo
//!   frame, or 32 SCK cycles per data-word."
//! * Pin 2 SD, p.9: "This pin tri-states when not actively driving the
//!   appropriate output channel. The SD trace should have a 100 kOhm pulldown
//!   resistor to discharge the line."
//! * Supply 1.62 to 3.63 V (Supply Voltage row -- NOT the 1.8..3.3 V
//!   characterisation condition at the head of that table).
//!
//! ⚠️ WHAT THIS MODEL DOES NOT KNOW. A microphone's output is sound, and there
//! is no sound in a simulator. The sample stream here is a SYNTHESISED tone
//! whose amplitude is driven by the `level` stimulus channel, and it is
//! labelled as such everywhere it surfaces. It answers "did the firmware clock
//! the bus correctly, on the right channel, and read plausible 24-bit words",
//! which is what an I2S bring-up actually fails at. It does not answer "does
//! the audio sound right", and nothing here should be read as if it did.

use crate::peripherals::device::I2sDevice;
use std::any::Any;

/// Which half of the stereo frame this part drives, selected by the L/R pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MicChannel {
    /// L/R tied low.
    Left,
    /// L/R tied high.
    Right,
}

/// Full-scale for a 24-bit two's-complement sample.
const FULL_SCALE_24: i32 = (1 << 23) - 1;

/// One period of the synthesised tone, in stereo frames. Deliberately a power
/// of two so the sequence is exactly reproducible and a test can name a sample
/// by index without floating-point drift creeping in.
const TONE_PERIOD_FRAMES: u32 = 64;

#[derive(Debug, serde::Serialize)]
pub struct Inmp441 {
    id: String,
    channel: MicChannel,
    /// 0..=100 %, driven by the `level` stimulus channel. 0 is a mic in a
    /// silent room, not a mic that is broken -- it still clocks out slots.
    level_pct: f64,
    /// Position in the synthesised tone, in frames.
    phase: u32,
    /// Slots this part actually drove, and slots it declined because they
    /// belonged to the other channel. Both are evidence: a firmware reading the
    /// wrong half sees only the second counter rise.
    driven_slots: u64,
    declined_slots: u64,
}

impl Inmp441 {
    pub fn new(id: impl Into<String>, channel: MicChannel) -> Self {
        Self {
            id: id.into(),
            channel,
            level_pct: 50.0,
            phase: 0,
            driven_slots: 0,
            declined_slots: 0,
        }
    }

    pub fn channel(&self) -> MicChannel {
        self.channel
    }

    pub fn driven_slots(&self) -> u64 {
        self.driven_slots
    }

    pub fn declined_slots(&self) -> u64 {
        self.declined_slots
    }

    /// The next 24-bit two's-complement sample, MSB-aligned into 32 bits.
    ///
    /// A triangle rather than a sine: it is exact in integer arithmetic, so the
    /// same firmware run produces the same bytes on every machine, which a
    /// float sine does not guarantee.
    fn next_sample_24(&mut self) -> i32 {
        let half = TONE_PERIOD_FRAMES / 2;
        let pos = self.phase % TONE_PERIOD_FRAMES;
        // Triangle in -1..=1 scaled by integer maths.
        let tri = if pos < half {
            (pos as i64 * 2 * FULL_SCALE_24 as i64) / half as i64 - FULL_SCALE_24 as i64
        } else {
            FULL_SCALE_24 as i64 - ((pos - half) as i64 * 2 * FULL_SCALE_24 as i64) / half as i64
        };
        self.phase = self.phase.wrapping_add(1);
        let scaled = (tri * self.level_pct.clamp(0.0, 100.0) as i64) / 100;
        scaled as i32
    }
}

impl I2sDevice for Inmp441 {
    fn next_slot(&mut self, right: bool) -> u32 {
        let mine = match self.channel {
            MicChannel::Left => !right,
            MicChannel::Right => right,
        };
        if !mine {
            // Datasheet p.9: SD tri-states when not driving its channel, and
            // the 100k pulldown discharges the line. Returning the sample here
            // would make a mic wired to the WRONG channel look like it works,
            // which is the single most common INMP441 bring-up mistake.
            self.declined_slots += 1;
            return 0;
        }
        self.driven_slots += 1;
        let s = self.next_sample_24();
        // 24-bit two's complement, MSB-aligned in a 32-bit slot: the low byte
        // is always zero on this part.
        ((s as u32) & 0x00FF_FFFF) << 8
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }

    fn artifacts(
        &self,
        id: &str,
        _opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        vec![crate::inspect::Artifact {
            kind: "i2s_microphone".to_string(),
            id: id.to_string(),
            meta: serde_json::json!({
                "channel": match self.channel { MicChannel::Left => "left", MicChannel::Right => "right" },
                "driven_slots": self.driven_slots,
                // The counter that catches a wrong-channel wiring: it rises
                // while `driven_slots` stays flat.
                "declined_slots": self.declined_slots,
                "level_pct": self.level_pct,
                "bits": 24,
                "encoding": "twos_complement_msb_aligned",
                // Stated in the evidence itself so nothing downstream can quote
                // these samples as if they were a recording.
                "signal": "synthesised triangle test tone, not real audio",
            }),
            bytes: None,
        }]
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Datasheet p.9: the part drives only its own half of the frame.
    #[test]
    fn a_left_mic_drives_only_the_left_slot() {
        let mut m = Inmp441::new("mic", MicChannel::Left);
        let left = m.next_slot(false);
        let right = m.next_slot(true);
        assert_ne!(left, 0, "left slot must carry a sample");
        assert_eq!(right, 0, "SD tri-states outside its channel");
        assert_eq!(m.driven_slots(), 1);
        assert_eq!(m.declined_slots(), 1);
    }

    #[test]
    fn a_right_mic_is_the_mirror() {
        let mut m = Inmp441::new("mic", MicChannel::Right);
        assert_eq!(m.next_slot(false), 0);
        assert_ne!(m.next_slot(true), 0);
    }

    /// A mic addressed on the wrong channel is SILENT, not absent. That is the
    /// distinction a bring-up needs: the bus is clocking, the part is there,
    /// and every word is zero.
    #[test]
    fn a_wrong_channel_read_is_silence_and_says_so() {
        let mut m = Inmp441::new("mic", MicChannel::Left);
        for _ in 0..32 {
            assert_eq!(m.next_slot(true), 0);
        }
        assert_eq!(m.driven_slots(), 0, "it never drove a slot");
        assert_eq!(m.declined_slots(), 32, "but it was asked 32 times");
    }

    /// 24-bit, MSB-aligned in 32: the low byte is always zero on this part.
    #[test]
    fn samples_are_24_bit_msb_aligned() {
        let mut m = Inmp441::new("mic", MicChannel::Left);
        for _ in 0..128 {
            assert_eq!(m.next_slot(false) & 0xFF, 0, "low byte must be empty");
        }
    }

    /// Level 0 is a silent room, not a dead part: it still answers on its own
    /// channel, with zero-valued samples.
    #[test]
    fn zero_level_still_clocks_slots() {
        let mut m = Inmp441::new("mic", MicChannel::Left);
        m.level_pct = 0.0;
        for _ in 0..8 {
            assert_eq!(m.next_slot(false), 0);
        }
        assert_eq!(m.driven_slots(), 8, "silence is still eight driven slots");
        assert_eq!(m.declined_slots(), 0);
    }

    /// The tone is integer-exact, so the same run gives the same bytes twice.
    #[test]
    fn the_sample_stream_is_reproducible() {
        let take = || {
            let mut m = Inmp441::new("mic", MicChannel::Left);
            (0..TONE_PERIOD_FRAMES * 2)
                .map(|_| m.next_slot(false))
                .collect::<Vec<_>>()
        };
        assert_eq!(take(), take());
    }

    /// It is a signal, not a constant -- a model that returned one value would
    /// pass every test above.
    #[test]
    fn the_tone_actually_varies() {
        let mut m = Inmp441::new("mic", MicChannel::Left);
        let s: Vec<u32> = (0..TONE_PERIOD_FRAMES)
            .map(|_| m.next_slot(false))
            .collect();
        let distinct = s.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(
            distinct > 8,
            "expected a varying tone, saw {distinct} distinct slots"
        );
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Inmp441Kit;
pub static INMP441_KIT: Inmp441Kit = Inmp441Kit;

static INMP441_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "inmp441",
    label: "INMP441 I2S Mic",
    summary: "Omnidirectional MEMS microphone, 24-bit I2S digital output.",
    detail: "TDK InvenSense INMP441 on a serial-audio bus. Answers in 32-bit channel slots, \
             24-bit two's complement MSB-aligned, and drives ONLY the half of the frame its \
             L/R pin selects -- the other half reads zero, because SD tri-states there and the \
             board's 100k pulldown wins. That is the part's commonest bring-up failure and the \
             model reproduces it rather than hiding it. On EFR32 the transport is a USART in \
             I2S mode (I2SCTRL.EN); the sample stream is a deterministic synthetic tone, not \
             real audio, and its artifact says so.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "channel",
        ty: ConfigType::Str,
        doc: "Which half of the stereo frame this mic drives: \"left\" (L/R tied low, \
                  the default) or \"right\" (L/R tied high). Datasheet pin 4. A mic asked \
                  for the other channel is SILENT, not absent.",
    }],
    labs: &[],
};

impl PeripheralKit for Inmp441Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &INMP441_METADATA
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let channel = match ctx.config_str("channel").unwrap_or("left") {
            "left" | "LEFT" | "Left" => MicChannel::Left,
            "right" | "RIGHT" | "Right" => MicChannel::Right,
            other => anyhow::bail!(
                "inmp441 '{}': channel '{}' is not a side of an I2S frame. Use \"left\" \
                 (L/R tied low) or \"right\" (L/R tied high) -- the mic drives exactly one.",
                ctx.device_id(),
                other,
            ),
        };
        let id = ctx.device_id().to_string();
        ctx.attach_i2s_device(Box::new(Inmp441::new(id, channel)))
    }
}
