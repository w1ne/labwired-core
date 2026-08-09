// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The ONE way a UART puts a real waveform on its TX pad.
//!
//! # Why this exists
//!
//! Nine UART models live in this engine and not one of them drives a pad. A
//! UART moves bytes through a FIFO and a status register, which is a faithful
//! model of everything firmware can observe — and it leaves the TX pin flat.
//! Serial is the single most probed signal in embedded work, so "clip the
//! analyzer to TX" answering with a straight line is the most visible hole the
//! instrument has.
//!
//! Like [`crate::peripherals::i2c_waveform`], the controller keeps its model
//! and *narrates* the framing its bytes imply. It already knows every fact the
//! waveform is made of — the byte, the word length, the parity setting, the
//! stop-bit count and the baud divisor — because those are exactly the
//! registers it models.
//!
//! # What this is and is not
//!
//! The waveform is **derived from the transfer, not measured from a wire**.
//! Framing, bit values, bit order, parity and the programmed baud rate are all
//! real, so a decoder recovers exactly the bytes the model sent. What it cannot
//! show is anything the byte-level model never had: break conditions, framing
//! errors from a mismatched baud rate, noise, or line-idle glitches. A UART
//! that grows a real bit engine should publish from it directly and stop
//! narrating.
//!
//! # Shape
//!
//! ```ignore
//! let mut wave = UartNarrator::new(TX_LINE, bit_time);
//! wave.frame(byte, UartFraming::default());
//! wave.emit_ending_at(&lines, now);
//! ```

use super::pad_lines::PadLines;
use super::wave_plan::{NarrationFit, WavePlan};

/// Parity as the control registers select it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parity {
    #[default]
    None,
    Even,
    Odd,
}

/// How one character is framed on the wire, straight from the control
/// registers. The defaults are 8N1 — what virtually every board boots to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UartFraming {
    /// Data bits per character (5..=9 on real parts; 8 is universal).
    pub data_bits: u8,
    pub parity: Parity,
    /// Stop bits. Half-bit stop lengths are not modelled; 1 or 2.
    pub stop_bits: u8,
}

impl UartFraming {
    /// Bit periods one character occupies: start + data + optional parity +
    /// stop. The ONE place this length is defined, so a pacing decision and the
    /// plan it paces can never disagree.
    pub fn frame_bits(&self) -> u64 {
        1 + u64::from(self.data_bits.clamp(5, 8))
            + u64::from(self.parity != Parity::None)
            + u64::from(self.stop_bits.clamp(1, 2))
    }
}

impl Default for UartFraming {
    fn default() -> Self {
        Self {
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
        }
    }
}

/// A planned UART waveform: the edge sequence a transmitted byte implies.
#[derive(Debug)]
pub struct UartNarrator {
    tx_line: usize,
    plan: WavePlan,
    cursor: u64,
}

impl UartNarrator {
    /// A plan for an idle line. UART idles HIGH (mark), so a start bit is
    /// always a falling edge — which is what a receiver, and a decoder reading
    /// this trace, synchronises on.
    ///
    /// `bit_time` is one bit period in engine cycles: the core clock divided by
    /// the programmed baud rate.
    pub fn new(tx_line: usize, bit_time: u64) -> Self {
        Self {
            tx_line,
            plan: WavePlan::new(&[true], bit_time),
            cursor: 0,
        }
    }

    /// A plan over a wire with more than one line (e.g. a port that also
    /// carries RX), so the TX index addresses the right one.
    pub fn with_lines(tx_line: usize, idle: &[bool], bit_time: u64) -> Self {
        Self {
            tx_line,
            plan: WavePlan::new(idle, bit_time),
            cursor: 0,
        }
    }

    /// One character: start bit (low), `data_bits` of `byte` LSB-first, an
    /// optional parity bit, then the stop bit(s) high.
    ///
    /// LSB-first is not a choice — it is what asynchronous serial does, and
    /// getting it backwards is the classic way to produce a trace that looks
    /// plausible and decodes to garbage.
    pub fn frame(&mut self, byte: u8, framing: UartFraming) {
        let bit = self.plan.bit_time();
        let data_bits = framing.data_bits.clamp(5, 8);

        // Start bit.
        self.plan.edge(self.tx_line, false, self.cursor);
        self.cursor += bit;

        let mut ones = 0u32;
        for index in 0..data_bits {
            let level = (byte >> index) & 1 != 0;
            if level {
                ones += 1;
            }
            self.plan.edge(self.tx_line, level, self.cursor);
            self.cursor += bit;
        }

        match framing.parity {
            Parity::None => {}
            Parity::Even => {
                self.plan.edge(self.tx_line, ones % 2 == 1, self.cursor);
                self.cursor += bit;
            }
            Parity::Odd => {
                self.plan.edge(self.tx_line, ones % 2 == 0, self.cursor);
                self.cursor += bit;
            }
        }

        // Stop bit(s): the line returns to mark and stays there.
        self.plan.edge(self.tx_line, true, self.cursor);
        self.cursor += bit * u64::from(framing.stop_bits.clamp(1, 2));
    }

    /// Total span of the plan in engine cycles.
    pub fn span(&self) -> u64 {
        self.plan.span()
    }

    /// Cycles the framed characters OCCUPY, stop bits included.
    ///
    /// Not the same as [`span`](Self::span), which is the last *edge*: a
    /// character whose top bits are all ones ends with no transition at all, so
    /// `0xFF` in 8N1 spans one bit period while occupying ten. Anything pacing
    /// one character against the next needs this, not the span.
    pub fn duration(&self) -> u64 {
        self.cursor
    }

    /// `true` when nothing has been framed.
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty()
    }

    /// One bit period in engine cycles.
    pub fn bit_time(&self) -> u64 {
        self.plan.bit_time()
    }

    /// Publish so the last edge lands on `end_cycle`. See [`NarrationFit`].
    #[must_use = "a compressed narration does not carry the programmed baud rate"]
    pub fn emit_ending_at(self, lines: &PadLines, end_cycle: u64) -> NarrationFit {
        self.plan.emit_ending_at(lines, end_cycle)
    }

    /// Publish ending at `end_cycle` without reaching back past `not_before` —
    /// the cycle an earlier flush of this same line already ran to. See
    /// [`WavePlan::emit_between`].
    #[must_use = "a compressed narration does not carry the programmed baud rate"]
    pub fn emit_between(self, lines: &PadLines, not_before: u64, end_cycle: u64) -> NarrationFit {
        self.plan.emit_between(lines, not_before, end_cycle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic_capture::LogicTap;

    const TX: usize = 0;
    const LINES: &[&str] = &["TX"];
    const BIT: u64 = 100;

    fn capture(wave: UartNarrator, end: u64) -> Vec<(u64, bool)> {
        let lines = PadLines::new(LINES, &[true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0]]);
        let _ = wave.emit_ending_at(&lines, end);
        let mut events: Vec<(u64, bool)> = tap
            .take_events()
            .iter()
            .map(|event| (event.cycle, event.value))
            .collect();
        events.sort_by_key(|&(cycle, _)| cycle);
        events
    }

    /// An INDEPENDENT async-serial decoder: find each falling edge from mark,
    /// then sample at the CENTRE of every following bit period — which is what
    /// a real receiver does and what a user reading the trace would do. It
    /// knows nothing about how the waveform was built.
    fn decode(events: &[(u64, bool)], bit: u64, framing: UartFraming) -> Vec<u8> {
        let level_at = |t: u64| -> bool {
            let mut level = true; // idle mark
            for &(cycle, value) in events {
                if cycle <= t {
                    level = value;
                } else {
                    break;
                }
            }
            level
        };
        let data_bits = u64::from(framing.data_bits.clamp(5, 8));
        let parity_bits = u64::from(framing.parity != Parity::None);
        let frame_bits = 1 + data_bits + parity_bits + u64::from(framing.stop_bits.clamp(1, 2));

        let mut out = Vec::new();
        let mut cursor = events.first().map(|&(c, _)| c).unwrap_or(0);
        let last = events.last().map(|&(c, _)| c).unwrap_or(0);
        while cursor <= last {
            // Synchronise: the next instant the line is low having been high.
            if level_at(cursor) {
                cursor += 1;
                continue;
            }
            let start = cursor;
            let mut byte = 0u8;
            for index in 0..data_bits {
                let centre = start + bit * (index + 1) + bit / 2;
                if level_at(centre) {
                    byte |= 1 << index;
                }
            }
            out.push(byte);
            cursor = start + bit * frame_bits;
        }
        out
    }

    #[test]
    fn a_narrated_character_decodes_back_to_the_byte_sent() {
        for byte in [0x00u8, 0x41, 0x55, 0xAA, 0xFF, 0x0D, 0x80, 0x01] {
            let mut wave = UartNarrator::new(TX, BIT);
            wave.frame(byte, UartFraming::default());
            let events = capture(wave, 100_000);
            assert_eq!(
                decode(&events, BIT, UartFraming::default()),
                vec![byte],
                "byte {byte:#04x} did not survive the wire",
            );
        }
    }

    #[test]
    fn a_string_decodes_in_order() {
        let mut wave = UartNarrator::new(TX, BIT);
        for &byte in b"Hi!\n" {
            wave.frame(byte, UartFraming::default());
        }
        let events = capture(wave, 1_000_000);
        assert_eq!(
            decode(&events, BIT, UartFraming::default()),
            b"Hi!\n".to_vec(),
        );
    }

    #[test]
    fn the_start_bit_is_a_falling_edge_from_idle_mark() {
        // A receiver syncs on this edge; idling low would make every frame
        // unsynchronisable.
        let mut wave = UartNarrator::new(TX, BIT);
        wave.frame(0xFF, UartFraming::default());
        let events = capture(wave, 100_000);
        assert!(!events.first().unwrap().1, "first transition must be low");
        assert!(events.last().unwrap().1, "the line returns to mark");
    }

    #[test]
    fn bits_are_sent_least_significant_first() {
        // 0x01 puts its only 1 in the FIRST data bit; MSB-first would put it
        // last. This is the mutation that makes a trace look right and decode
        // to nonsense.
        let mut wave = UartNarrator::new(TX, BIT);
        wave.frame(0x01, UartFraming::default());
        let events = capture(wave, 100_000);
        let first_data_centre = events.first().unwrap().0 + BIT + BIT / 2;
        let level_at = |t: u64| {
            let mut level = true;
            for &(c, v) in &events {
                if c <= t {
                    level = v;
                } else {
                    break;
                }
            }
            level
        };
        assert!(level_at(first_data_centre), "bit 0 of 0x01 is 1");
    }

    #[test]
    fn every_bit_lasts_exactly_one_programmed_period() {
        // Measuring the trace must give the baud rate the divisor programs.
        let mut wave = UartNarrator::new(TX, BIT);
        wave.frame(0x55, UartFraming::default()); // alternating → an edge per bit
        let events = capture(wave, 100_000);
        let cycles: Vec<u64> = events.iter().map(|&(c, _)| c).collect();
        // 0x55 alternates on every data bit, so consecutive transitions are
        // exactly ONE bit period apart. Asserting only that the gap is a
        // multiple of BIT would pass at half or double the programmed baud,
        // which is the error this test exists to catch.
        for pair in cycles.windows(2) {
            assert_eq!(
                pair[1] - pair[0],
                BIT,
                "every bit lasts exactly one programmed period: {cycles:?}",
            );
        }
    }

    #[test]
    fn even_parity_makes_the_ones_count_even() {
        let mut wave = UartNarrator::new(TX, BIT);
        let framing = UartFraming {
            parity: Parity::Even,
            ..Default::default()
        };
        wave.frame(0x03, framing); // two ones → parity 0
        let events = capture(wave, 100_000);
        let start = events.first().unwrap().0;
        let level_at = |t: u64| {
            let mut level = true;
            for &(c, v) in &events {
                if c <= t {
                    level = v;
                } else {
                    break;
                }
            }
            level
        };
        let parity_centre = start + BIT * 9 + BIT / 2;
        assert!(!level_at(parity_centre), "even parity of 0x03 is 0");
    }

    #[test]
    fn odd_parity_is_the_complement() {
        let mut wave = UartNarrator::new(TX, BIT);
        let framing = UartFraming {
            parity: Parity::Odd,
            ..Default::default()
        };
        wave.frame(0x03, framing);
        let events = capture(wave, 100_000);
        let start = events.first().unwrap().0;
        let level_at = |t: u64| {
            let mut level = true;
            for &(c, v) in &events {
                if c <= t {
                    level = v;
                } else {
                    break;
                }
            }
            level
        };
        assert!(
            level_at(start + BIT * 9 + BIT / 2),
            "odd parity of 0x03 is 1"
        );
    }

    #[test]
    fn two_stop_bits_hold_the_line_longer_before_the_next_start() {
        let one = {
            let mut w = UartNarrator::new(TX, BIT);
            w.frame(0x00, UartFraming::default());
            w.frame(0x00, UartFraming::default());
            w.span()
        };
        let two = {
            let mut w = UartNarrator::new(TX, BIT);
            let f = UartFraming {
                stop_bits: 2,
                ..Default::default()
            };
            w.frame(0x00, f);
            w.frame(0x00, f);
            w.span()
        };
        assert_eq!(two - one, BIT, "one extra stop bit before the second frame");
    }

    #[test]
    fn a_transfer_with_no_history_behind_it_still_decodes() {
        // Same guarantee as I²C: the timebase can be lost, the bytes cannot.
        let mut wave = UartNarrator::new(TX, 10_000);
        wave.frame(0x41, UartFraming::default());
        let lines = PadLines::new(LINES, &[true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0]]);
        let fit = wave.emit_ending_at(&lines, 40);
        assert!(matches!(fit, NarrationFit::Compressed { .. }), "{fit:?}");
        // The point is not that SOME trace survives — it is that the CHARACTER
        // does. Compression packs to one cycle per transition, so the bit
        // period is gone and no centre-sampling decoder can read it; what must
        // be intact is the sequence of levels, because that is what carries the
        // bits. Compare against the same frame emitted with room to breathe: a
        // compression that dropped or reordered a transition shows up here.
        let compressed: Vec<bool> = tap.take_events().iter().map(|e| e.value).collect();
        assert!(
            !compressed.is_empty(),
            "a compressed frame still leaves a trace"
        );

        let mut roomy = UartNarrator::new(TX, 10_000);
        roomy.frame(0x41, UartFraming::default());
        let roomy_lines = PadLines::new(LINES, &[true]);
        let roomy_tap = LogicTap::new();
        roomy_lines.install_tap(Some(roomy_tap.clone()), vec![vec![0]]);
        assert_eq!(
            roomy.emit_ending_at(&roomy_lines, 1_000_000),
            NarrationFit::Exact,
        );
        let exact: Vec<bool> = roomy_tap.take_events().iter().map(|e| e.value).collect();

        assert_eq!(
            compressed, exact,
            "every transition survives compression, in order: the timebase is \
             lost, the bits are not",
        );
    }

    #[test]
    fn publishing_without_an_armed_tap_still_moves_the_level() {
        // A frame always ENDS at mark, which is also where the line idles — so
        // asserting the level after a whole character proves nothing at all.
        // Publish a truncated waveform that leaves the line LOW, which it can
        // only be if the narration actually drove it.
        let lines = PadLines::new(LINES, &[true]);
        assert!(
            lines.level(TX),
            "idles at mark before anything is published"
        );
        let mut plan = WavePlan::new(&[true], BIT);
        plan.edge(TX, false, BIT); // a start bit with no character after it
        plan.emit_starting_at(&lines, 0);
        assert!(
            !lines.level(TX),
            "the level moves with no tap armed: publication is not tap-gated",
        );
    }
}
