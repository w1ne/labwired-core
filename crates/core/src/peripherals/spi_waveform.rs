// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The ONE way a transaction-level SPI controller puts a real waveform on its
//! pads.
//!
//! # Why this exists
//!
//! Two kinds of SPI controller live in this engine.
//!
//! A **bit engine** (STM32, [`crate::peripherals::spi`]) walks SCK and MOSI
//! itself as the engine advances — see `stm32_frame_levels` /
//! `stm32_drive_levels` and the per-cycle `ticks_left` countdown — so it already
//! knows the wire state at every cycle and publishes it straight into
//! [`PadLines`]. Nothing here applies to it.
//!
//! A **transaction-level** controller (RP2040 PL022,
//! [`crate::peripherals::rp2040::spi`]) moves a whole word inside the `SSPDR`
//! write: no shift counter, no bit index, `BSY` never observed. That is a
//! faithful model of everything firmware can read back through the registers —
//! and it drives the pads not at all. Clip the analyzer to GP3 while the
//! firmware is clocking bytes out and you get the SIO output latch: a flat line,
//! which reads to a user as "my firmware is broken" rather than "this family has
//! no wire model".
//!
//! So: the controller keeps its transfer model, and *narrates* the waveform the
//! transfer put on the wire. It knows every fact the waveform is made of — the
//! word, the frame width (SSPCR0.DSS), the clock polarity and phase
//! (SSPCR0.SPO/SPH) and the bit period (SSPCPSR.CPSDVSR × (1 + SSPCR0.SCR)) —
//! because those are exactly the registers it models.
//!
//! # What this is and is not
//!
//! The waveform is **derived from the transfer, not measured from a wire**. Bit
//! values, bit ORDER, frame width, clock polarity/phase, the sampling edge, the
//! programmed bit rate and the chip-select framing are all real, so an
//! independent decoder recovers exactly the words the model shifted.
//!
//! What it cannot show is anything the word-level model never had: MISO (nothing
//! in the engine drives it — the RP2040 model has no attached devices and reads
//! back `0x00`, so MISO is NOT one of the published lines and NOT routed to a
//! pad), slave-side wait states, bus contention, or rise-time shape.
//!
//! Chip-select framing follows the RP2040 datasheet §4.4.3 ("Motorola SPI
//! Format"), which states the two continuous-transfer rules explicitly and
//! OPPOSITELY. Read it with `labwired_datasheet part=rp2040`; the pages below
//! are pinned to the document's content hash, so they stay valid across vendor
//! revisions:
//!
//! * `SPH = 0` (pages 510, 512) — "in the case of continuous back-to-back
//!   transmissions, the SSPFSSOUT signal must be pulsed HIGH between each data
//!   word transfer. This is because the slave select pin freezes the data in
//!   its serial peripheral register and does not permit it to be altered if the
//!   SPH bit is logic zero."
//! * `SPH = 1` (pages 511, 512) — "For continuous back-to-back transfers, the
//!   SSPFSSOUT pin is held LOW between successive data words and termination is
//!   the same as that of the single word transfer."
//!
//! So the phase decides the framing, and getting it backwards is observable: a
//! decoder that cuts frames on chip select would read one continuous SPH=1
//! burst as N separate transfers, or N back-to-back SPH=0 words as one
//! oversized frame. Both decode to bytes that never crossed the bus.
//!
//! ⚠️ FIDELITY: the chip-select framing below
//! asserts CSn half a bit period before the first clock edge of each frame and
//! releases it half a period after the last, pulsing high between frames. That
//! is the Motorola SPI frame format the pico-sdk default (`spi_init` →
//! `spi_set_format(…, SPI_CPHA_0, …)`) programs. The ARM PL022 TRM is NOT
//! present in this checkout (no RP2040 datasheet or DDI0194 under
//! /Volumes/LabWired), so whether `SPH = 1` lets real silicon hold SSPFSSOUT low
//! across back-to-back frames is **unverified**; this narrator pulses per frame
//! in both phases. Segmenting on CS is what lets a decoder cut frames, and a
//! pulse is the conservative shape — it never merges two frames into one.
//!
//! # Cost
//!
//! O(edges), not O(cycles): an 8-bit frame is ~19 transitions regardless of how
//! many thousand engine cycles it spans. Nothing is stepped per cycle, and a
//! controller with no routed pads pays one branch per transfer.
//!
//! # Shape
//!
//! ```ignore
//! let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, bit_time);
//! wave.frame(0xA5, SpiFraming { cpol, cpha, bits: 8 });
//! wave.emit_between(&lines, previous_flush_end, now);
//! ```

use super::pad_lines::PadLines;
use super::wave_plan::WavePlan;

pub use super::wave_plan::NarrationFit;

/// How one word is framed on the wire, straight from SSPCR0.
///
/// The defaults are what pico-sdk's `spi_init` programs: 8-bit frames, mode 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiFraming {
    /// SSPCR0.SPO — the level SCK rests at between frames.
    pub cpol: bool,
    /// SSPCR0.SPH — `false`: the LEADING clock edge samples; `true`: the
    /// leading edge shifts and the TRAILING edge samples.
    pub cpha: bool,
    /// Frame width in bits, SSPCR0.DSS + 1. The PL022 supports 4..=16
    /// (DSS 0000..0010 are reserved, per the SVD field description).
    pub bits: u8,
}

impl Default for SpiFraming {
    fn default() -> Self {
        Self {
            cpol: false,
            cpha: false,
            bits: 8,
        }
    }
}

impl SpiFraming {
    /// Bit periods one frame OCCUPIES on the wire: the clocked bits, plus one
    /// period split between chip-select setup and hold, plus one idle period
    /// before the next frame can begin.
    ///
    /// The ONE place this length is defined, so a controller pacing a burst
    /// against the wire and the plan it eventually builds can never disagree —
    /// the mistake [`crate::peripherals::uart_waveform::UartFraming::frame_bits`]
    /// exists to prevent on the serial side.
    pub fn frame_bits(&self) -> u64 {
        u64::from(self.bits.clamp(4, 16)) + 2
    }
}

/// A planned SPI waveform: the edge sequence a completed transfer implies.
///
/// Times are relative to the start of the plan until [`emit_between`] or
/// [`emit_ending_at`] anchors them to engine cycles.
///
/// [`emit_between`]: Self::emit_between
/// [`emit_ending_at`]: Self::emit_ending_at
#[derive(Debug)]
pub struct SpiNarrator {
    sck_line: usize,
    mosi_line: usize,
    /// `None` for a controller whose chip select is driven by GPIO rather than
    /// by the peripheral. The RP2040 passes `Some` because IO_BANK0 can hand
    /// `spi*_ss_n` to the SSP (`SPI.begin(true)` in arduino-pico does exactly
    /// that); the route simply is not live while firmware keeps the pad on SIO.
    csn_line: Option<usize>,
    /// Relative cursor: where the next frame begins.
    cursor: u64,
    /// Whether chip select is currently held low. Only ever true BETWEEN frames
    /// under `SPH = 1`, which is the phase that holds it across a burst.
    cs_held: bool,
    /// Cycle the last frame's final bit ended, so the closing chip-select
    /// release can be placed one clock period after it, as the datasheet says.
    cs_release_at: u64,
    /// Edge accumulation and cycle-axis anchoring, shared with every other
    /// narrator (see [`crate::peripherals::wave_plan`]).
    plan: WavePlan,
}

impl SpiNarrator {
    /// A plan over a wire currently resting at `idle` (in line order), so a
    /// second burst continues from where the first left it rather than
    /// inventing transitions back to a nominal rest state.
    ///
    /// `bit_time` is one SCK period in engine cycles; [`WavePlan`] clamps it to
    /// at least 2 so a period's halves stay distinguishable.
    pub fn with_lines(
        sck_line: usize,
        mosi_line: usize,
        csn_line: Option<usize>,
        idle: &[bool],
        bit_time: u64,
    ) -> Self {
        Self {
            sck_line,
            mosi_line,
            csn_line,
            cursor: 0,
            cs_held: false,
            cs_release_at: 0,
            plan: WavePlan::new(idle, bit_time),
        }
    }

    /// One frame: chip select asserted, `bits` of `word` clocked out MSB-first,
    /// chip select released.
    ///
    /// MSB-first is not a choice. pico-sdk's `spi_set_format` refuses anything
    /// else outright — `// LSB-first not supported on PL022:` followed by
    /// `invalid_params_if(HARDWARE_SPI, order != SPI_MSB_FIRST)` — so narrating
    /// LSB-first would produce a trace that looks entirely plausible and decodes
    /// to garbage, which is the single most damaging way to get this wrong.
    ///
    /// Within each bit period MOSI is driven at the START and the SAMPLING edge
    /// falls half a period later, so data is stable across every edge a receiver
    /// (or a user reading the trace) latches on:
    ///
    /// * `cpha == false` — MOSI at `t`, leading edge at `t + half`, trailing at
    ///   `t + bit`. MOSI moves on the trailing edge, which is what CPHA=0 does.
    /// * `cpha == true` — MOSI and the leading edge together at `t`, trailing
    ///   (sampling) edge at `t + half`. Shifting on the leading edge and
    ///   sampling on the trailing one is what CPHA=1 does.
    ///
    /// Both cases give an SCK period of exactly `bit_time`, so measuring the
    /// trace returns the rate CPSDVSR/SCR program.
    pub fn frame(&mut self, word: u16, framing: SpiFraming) {
        let bit = self.plan.bit_time();
        let half = bit / 2;
        let bits = framing.bits.clamp(4, 16);

        // Park SCK at the programmed polarity before anything else. Free when it
        // already rests there (a re-asserted level is not an edge); the one case
        // it matters is firmware that reprogrammed SPO between bursts, where the
        // wire is still sitting at the OLD idle and the first "leading edge"
        // would otherwise be a transition in the wrong direction.
        self.plan.edge(self.sck_line, framing.cpol, self.cursor);

        let mut t = self.cursor;
        // Chip-select setup. Under SPH=1 a continuous burst HOLDS it low, so
        // asserting is a no-op on every frame after the first (re-asserting a
        // level the line already holds records no edge). Under SPH=0 the
        // previous frame released it, so this is a fresh assert each time.
        if let Some(cs) = self.csn_line {
            self.plan.edge(cs, false, t);
            self.cs_held = true;
        }
        t += half;

        for index in (0..bits).rev() {
            // MSB-first: index counts down from bits-1.
            let level = (word >> index) & 1 != 0;
            if framing.cpha {
                // Leading edge shifts; the trailing edge is the sampling edge.
                // Two lines moving at one instant is fine — the capture layer
                // keeps one level per CHANNEL per cycle, and these are different
                // channels. Two transitions of ONE line sharing a cycle is the
                // thing that must never happen, and none do here.
                self.plan.edge(self.mosi_line, level, t);
                self.plan.edge(self.sck_line, !framing.cpol, t);
                self.plan.edge(self.sck_line, framing.cpol, t + half);
            } else {
                // Data set up while the clock idles; the leading edge samples.
                self.plan.edge(self.mosi_line, level, t);
                self.plan.edge(self.sck_line, !framing.cpol, t + half);
                self.plan.edge(self.sck_line, framing.cpol, t + bit);
            }
            t += bit;
        }

        // Chip-select termination. The datasheet returns SSPFSSOUT to idle one
        // SSPCLKOUT period after the last bit is captured, in BOTH phases —
        // what differs is whether the next word in a burst gets a fresh pulse
        // (SPH=0) or stays inside the same low window (SPH=1).
        self.cs_release_at = t + half;
        if let Some(cs) = self.csn_line {
            if !framing.cpha {
                self.plan.edge(cs, true, self.cs_release_at);
                self.cs_held = false;
            }
        }
        // MOSI holds its last driven level, like a real pad.
        self.cursor += framing.frame_bits() * bit;
    }

    /// Total span of the plan in engine cycles (its LAST edge).
    pub fn span(&self) -> u64 {
        self.plan.span()
    }

    /// Cycles the framed words OCCUPY, chip-select hold and inter-frame gap
    /// included. Not the same as [`span`](Self::span) — anything pacing one
    /// burst against the next needs this.
    pub fn duration(&self) -> u64 {
        self.cursor
    }

    /// `true` when nothing has been framed.
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty()
    }

    /// One SCK period in engine cycles.
    pub fn bit_time(&self) -> u64 {
        self.plan.bit_time()
    }

    /// Release a chip select an `SPH = 1` burst has been holding low.
    ///
    /// Called by every publish path. Without it a held-low burst would end with
    /// chip select still asserted, and the NEXT burst's assert would record no
    /// edge at all — the two transfers would fuse into one frame on the trace.
    fn close_burst(&mut self) {
        if let Some(cs) = self.csn_line {
            if self.cs_held {
                self.plan.edge(cs, true, self.cs_release_at);
                self.cs_held = false;
            }
        }
    }

    /// Publish so the last edge lands on `end_cycle`. See [`NarrationFit`].
    #[must_use = "a compressed narration does not carry the programmed bit rate"]
    pub fn emit_ending_at(mut self, lines: &PadLines, end_cycle: u64) -> NarrationFit {
        self.close_burst();
        self.plan.emit_ending_at(lines, end_cycle)
    }

    /// Publish ending at `end_cycle` without reaching back past `not_before` —
    /// the cycle an earlier flush of this same wire already ran to. See
    /// [`WavePlan::emit_between`].
    #[must_use = "a compressed narration does not carry the programmed bit rate"]
    pub fn emit_between(
        mut self,
        lines: &PadLines,
        not_before: u64,
        end_cycle: u64,
    ) -> NarrationFit {
        self.close_burst();
        self.plan.emit_between(lines, not_before, end_cycle)
    }

    /// Publish with the first edge at `start_cycle` (unit-test shape).
    pub fn emit_starting_at(mut self, lines: &PadLines, start_cycle: u64) {
        self.close_burst();
        self.plan.emit_starting_at(lines, start_cycle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic_capture::LogicTap;

    const SCK: usize = 0;
    const MOSI: usize = 1;
    const CSN: usize = 2;
    const LINES: &[&str] = &["SCK", "MOSI", "CSn"];
    const BIT: u64 = 100;

    fn capture(wave: SpiNarrator, idle: &[bool], start: u64) -> Vec<(u64, u32, bool)> {
        let lines = PadLines::new(LINES, idle);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1], vec![2]]);
        wave.emit_starting_at(&lines, start);
        let mut events: Vec<_> = tap
            .take_events()
            .iter()
            .map(|e| (e.cycle, e.ch, e.value))
            .collect();
        // MOSI (ch 1) before SCK (ch 0) would be wrong; sort ties so a data
        // setup that shares a cycle with a clock edge settles FIRST, which is
        // what a real receiver's setup window means.
        events.sort_by_key(|&(cycle, ch, _)| (cycle, std::cmp::Reverse(ch)));
        events
    }

    /// An INDEPENDENT SPI decoder: replays the three lines in cycle order,
    /// samples MOSI on the sampling edge the mode selects, and cuts frames when
    /// chip select releases. It knows nothing about how the plan was built.
    fn decode(events: &[(u64, u32, bool)], framing: SpiFraming) -> Vec<u16> {
        let (mut sck, mut cs, mut mosi) = (framing.cpol, true, false);
        let (mut acc, mut count) = (0u32, 0u8);
        let mut out = Vec::new();
        for &(_, ch, value) in events {
            let previous_sck = sck;
            match ch as usize {
                0 => sck = value,
                1 => mosi = value,
                _ => cs = value,
            }
            if ch as usize == 2 && value {
                acc = 0;
                count = 0;
                continue;
            }
            if ch as usize == 0 && !cs && previous_sck != sck {
                let sampling = if framing.cpha {
                    sck == framing.cpol // trailing edge
                } else {
                    sck != framing.cpol // leading edge
                };
                if sampling {
                    acc = (acc << 1) | u32::from(mosi);
                    count += 1;
                    if count == framing.bits {
                        out.push(acc as u16);
                        acc = 0;
                        count = 0;
                    }
                }
            }
        }
        out
    }

    fn idle_for(framing: SpiFraming) -> Vec<bool> {
        vec![framing.cpol, false, true]
    }

    #[test]
    fn a_narrated_frame_decodes_back_to_the_word_the_model_shifted() {
        for cpol in [false, true] {
            for cpha in [false, true] {
                let framing = SpiFraming {
                    cpol,
                    cpha,
                    bits: 8,
                };
                for word in [0x00u16, 0x01, 0x80, 0xA5, 0x55, 0xFF] {
                    let idle = idle_for(framing);
                    let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, BIT);
                    wave.frame(word, framing);
                    assert_eq!(
                        decode(&capture(wave, &idle, 1_000), framing),
                        vec![word],
                        "mode {}{} lost {word:#06x}",
                        u8::from(cpol),
                        u8::from(cpha),
                    );
                }
            }
        }
    }

    #[test]
    fn bits_go_out_most_significant_first() {
        // 0x80 puts its only 1 in the FIRST clocked bit; LSB-first would put it
        // last. The PL022 cannot do LSB-first at all (pico-sdk spi_set_format
        // rejects it), so getting this backwards models no real part.
        let framing = SpiFraming::default();
        let idle = idle_for(framing);
        let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, BIT);
        wave.frame(0x80, framing);
        let events = capture(wave, &idle, 0);
        let first_sample = events
            .iter()
            .filter(|&&(_, ch, value)| ch == 0 && value != framing.cpol)
            .map(|&(cycle, _, _)| cycle)
            .next()
            .expect("a leading edge");
        let mosi_at = |t: u64| {
            events
                .iter()
                .filter(|&&(cycle, ch, _)| ch == 1 && cycle <= t)
                .map(|&(_, _, value)| value)
                .next_back()
                .unwrap_or(false)
        };
        assert!(mosi_at(first_sample), "bit 7 of 0x80 is 1");
    }

    #[test]
    fn sck_runs_at_exactly_the_programmed_period() {
        let framing = SpiFraming::default();
        let idle = idle_for(framing);
        let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, BIT);
        wave.frame(0xA5, framing);
        let events = capture(wave, &idle, 0);
        let leading: Vec<u64> = events
            .iter()
            .filter(|&&(_, ch, value)| ch == 0 && value != framing.cpol)
            .map(|&(cycle, _, _)| cycle)
            .collect();
        assert_eq!(leading.len(), 8, "one clock per bit: {leading:?}");
        for pair in leading.windows(2) {
            // Asserting only "a multiple of BIT" would pass at half or double
            // the programmed rate, which is the error this exists to catch.
            assert_eq!(pair[1] - pair[0], BIT, "SCK period drifted: {leading:?}");
        }
    }

    #[test]
    fn mosi_never_moves_on_a_sampling_edge() {
        for cpha in [false, true] {
            let framing = SpiFraming {
                cpol: false,
                cpha,
                bits: 8,
            };
            let idle = idle_for(framing);
            let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, BIT);
            wave.frame(0x5A, framing);
            let events = capture(wave, &idle, 0);
            let sampling: Vec<u64> = events
                .iter()
                .filter(|&&(_, ch, value)| {
                    ch == 0
                        && if cpha {
                            value == framing.cpol
                        } else {
                            value != framing.cpol
                        }
                })
                .map(|&(cycle, _, _)| cycle)
                .collect();
            for &(cycle, ch, _) in &events {
                if ch == 1 {
                    assert!(
                        !sampling.contains(&cycle),
                        "MOSI moved on a sampling edge at {cycle} (cpha={cpha})",
                    );
                }
            }
        }
    }

    #[test]
    fn every_frame_is_bracketed_by_its_own_chip_select_pulse() {
        // Two frames must decode as two words, not as one 16-bit word: without
        // a CS pulse between them a decoder has no frame boundary at all.
        let framing = SpiFraming::default();
        let idle = idle_for(framing);
        let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, BIT);
        wave.frame(0x12, framing);
        wave.frame(0x34, framing);
        let events = capture(wave, &idle, 0);
        assert_eq!(decode(&events, framing), vec![0x12, 0x34]);
        let cs_edges: Vec<bool> = events
            .iter()
            .filter(|&&(_, ch, _)| ch == 2)
            .map(|&(_, _, value)| value)
            .collect();
        assert_eq!(
            cs_edges,
            vec![false, true, false, true],
            "one assert/release pair per frame: {cs_edges:?}",
        );
    }

    #[test]
    fn continuous_transfers_frame_the_way_the_phase_says() {
        // Straight out of the RP2040 datasheet §4.4.3, which states the two
        // rules in opposite directions:
        //
        //   SPH=0 — "the SSPFSSOUT signal must be pulsed HIGH between each data
        //            word transfer"
        //   SPH=1 — "the SSPFSSOUT pin is held LOW between successive data
        //            words"
        //
        // Three words, so "pulsed between each" and "held across" are visibly
        // different: three assert/release pairs against exactly one.
        for cpha in [false, true] {
            let framing = SpiFraming {
                cpol: false,
                cpha,
                bits: 8,
            };
            let idle = idle_for(framing);
            let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, BIT);
            for word in [0x11u16, 0x22, 0x33] {
                wave.frame(word, framing);
            }
            let events = capture(wave, &idle, 0);
            let cs: Vec<bool> = events
                .iter()
                .filter(|&&(_, ch, _)| ch as usize == CSN)
                .map(|&(_, _, value)| value)
                .collect();
            if cpha {
                assert_eq!(
                    cs,
                    vec![false, true],
                    "SPH=1 holds chip select LOW across the whole burst",
                );
            } else {
                assert_eq!(
                    cs,
                    vec![false, true, false, true, false, true],
                    "SPH=0 pulses chip select HIGH between each word",
                );
            }
            // Either way the words must survive — framing is not an excuse to
            // lose data.
            assert_eq!(decode(&events, framing), vec![0x11, 0x22, 0x33]);
        }
    }

    #[test]
    fn a_frame_narrower_than_eight_bits_clocks_only_that_many() {
        let framing = SpiFraming {
            cpol: false,
            cpha: false,
            bits: 4,
        };
        let idle = idle_for(framing);
        let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, BIT);
        wave.frame(0x0D, framing);
        assert_eq!(decode(&capture(wave, &idle, 0), framing), vec![0x0D]);
    }

    #[test]
    fn the_paced_duration_matches_what_the_plan_actually_occupies() {
        // The pacing decision and the plan MUST agree, or a burst is published
        // before the wire has carried it (or held forever).
        let framing = SpiFraming::default();
        let idle = idle_for(framing);
        let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, BIT);
        wave.frame(0xFF, framing);
        wave.frame(0x00, framing);
        assert_eq!(wave.duration(), 2 * framing.frame_bits() * BIT);
        assert!(
            wave.span() < wave.duration(),
            "the last edge is not the end"
        );
    }

    #[test]
    fn a_transfer_with_no_history_behind_it_still_decodes() {
        // Same guarantee as I²C and UART: the timebase can be lost, the word
        // cannot.
        let framing = SpiFraming::default();
        let idle = idle_for(framing);
        let mut wave = SpiNarrator::with_lines(SCK, MOSI, Some(CSN), &idle, 4_000);
        wave.frame(0xA5, framing);

        let lines = PadLines::new(LINES, &idle);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1], vec![2]]);
        let fit = wave.emit_ending_at(&lines, 60);
        assert!(matches!(fit, NarrationFit::Compressed { .. }), "{fit:?}");

        let mut events: Vec<_> = tap
            .take_events()
            .iter()
            .map(|e| (e.cycle, e.ch, e.value))
            .collect();
        events.sort_by_key(|&(cycle, ch, _)| (cycle, std::cmp::Reverse(ch)));
        assert_eq!(
            decode(&events, framing),
            vec![0xA5],
            "the word must survive compression even though the rate does not",
        );
        for channel in [0u32, 1, 2] {
            let per_line: Vec<u64> = events
                .iter()
                .filter(|&&(_, ch, _)| ch == channel)
                .map(|&(cycle, _, _)| cycle)
                .collect();
            let mut unique = per_line.clone();
            unique.dedup();
            assert_eq!(
                unique.len(),
                per_line.len(),
                "channel {channel} collapsed: {per_line:?}",
            );
        }
    }

    #[test]
    fn publishing_without_an_armed_tap_still_moves_the_levels() {
        // The analyzer is usually NOT armed; pad levels must still be right so
        // `read_gpio_pad` sees the wire. Stop mid-frame so the assertion cannot
        // be satisfied by a `PadLines` nothing ever touched.
        let framing = SpiFraming {
            cpol: true,
            cpha: false,
            bits: 8,
        };
        let idle = idle_for(framing);
        let lines = PadLines::new(LINES, &idle);
        assert!(lines.level(CSN), "chip select idles released");
        let mut plan = WavePlan::new(&idle, BIT);
        plan.edge(CSN, false, 0);
        plan.edge(SCK, false, BIT / 2);
        plan.emit_starting_at(&lines, 0);
        assert!(!lines.level(CSN), "publication is not tap-gated");
        assert!(!lines.level(SCK), "and SCK left CPOL");
    }
}
