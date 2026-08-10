//! The ONE way a transaction-level I²C controller puts a real waveform on its
//! pads.
//!
//! # Why this exists
//!
//! Two kinds of I²C controller live in this engine.
//!
//! A **bit engine** (ESP32-C3, [`crate::peripherals::esp32c3::i2c`]) walks SCL
//! and SDA itself as the engine advances, so it already knows the wire state at
//! every cycle and publishes it straight into [`PadLines`]. Nothing here
//! applies to it.
//!
//! A **transaction-level** controller (STM32 [`crate::peripherals::i2c`],
//! ESP32-S3, classic ESP32, RP2040) models the bus one *phase* at a time: it
//! captures the addressed slave, exchanges the byte through the device model,
//! and charges the phase a TIMINGR-derived countdown. That is a faithful model
//! of everything firmware can observe through the registers — and it drives the
//! pads not at all. Clip a logic analyzer to those pins and you get a flat
//! line, which reads to a user as "my firmware is broken" rather than "this
//! family has no wire model". Making every such controller grow its own bit
//! engine would be four rewrites of timing that is currently pinned against
//! silicon captures.
//!
//! So: the controller keeps its phase model, and *narrates* the waveform that
//! phase put on the wire. It already knows every fact the waveform is made of
//! — the address, the direction, the data byte, whether the slave ACKed, and
//! the SCL bit time — because those are exactly what it modelled. This module
//! turns those facts into the edge sequence they imply, stamped at the cycles
//! they occupied, via [`PadLines::set_line_at`].
//!
//! # What this is and is not
//!
//! The waveform is **derived from the transaction, not measured from a wire**.
//! What it gets right is everything the transaction determines: bit rate, frame
//! boundaries, START/STOP conditions, bit values, ACK/NACK, and data validity
//! around every SCL rising edge — so a decoder (the in-engine analyzer, or a
//! user reading the trace) recovers exactly the bytes the model exchanged.
//!
//! What it cannot show is anything the phase model never had: clock stretching
//! by the slave, arbitration loss between multiple masters, bus contention,
//! glitches, or rise-time shape. A bit engine is still the higher-fidelity
//! answer, and a family that grows one should publish from it directly and stop
//! narrating. Until then this is the difference between a measurable bus and a
//! flat line, and it is honest about which bits of it are real.
//!
//! # Cost
//!
//! O(edges), not O(cycles): a frame is ~20 transitions regardless of how many
//! thousands of engine cycles it spans. Nothing is stepped per cycle, and a
//! controller with no analyzer armed pays only the level updates.
//!
//! # Shape
//!
//! Build the frames as a plan in *relative* time, then anchor the finished plan
//! to real engine cycles:
//!
//! ```ignore
//! let mut wave = I2cNarrator::new(LINE_SCL, LINE_SDA, bit_time);
//! wave.start();
//! wave.frame(address << 1 | read_bit, acked);
//! wave.emit_ending_at(&lines, phase_end_cycle);
//! ```
//!
//! Anchoring at the END (rather than stretching the plan to fill the phase
//! window) is deliberate: it preserves the true SCL period, so a user measuring
//! the bit rate on the analyzer reads the frequency TIMINGR actually programs.
//! A plan slightly longer than its phase window simply starts slightly earlier,
//! overlapping idle bus time where both lines were already high.
//!
//! # When the run is younger than the waveform
//!
//! That backward reach needs somewhere to go. Firmware that boots, sets up
//! clocks and pins and only then touches a bus always has the room; a transfer
//! fired in the opening cycles of a run does not, and a waveform stamped past
//! the present would be clamped by the capture layer onto one cycle — a
//! transaction rendered as a spike.
//!
//! [`I2cNarrator::emit_ending_at`] therefore compresses instead: relative times
//! are scaled into the cycles that ARE available, preserving edge order, the
//! grouping of simultaneous edges, and a distinct cycle for every transition of
//! a given line. Frame count, bit values and ACKs all survive, so the trace
//! still decodes to the bytes that crossed the bus — only the timebase is no
//! longer the programmed one. It reports which case applied via
//! [`NarrationFit`], and degrades to [`NarrationFit::LevelsOnly`] (net levels,
//! no trace) rather than inventing timing it cannot represent.

use super::pad_lines::PadLines;
use super::wave_plan::WavePlan;

pub use super::wave_plan::NarrationFit;

/// A planned I²C waveform: the edge sequence a completed transaction implies.
///
/// Times are relative to the start of the plan until [`emit_ending_at`] or
/// [`emit_starting_at`] anchors them to engine cycles.
///
/// [`emit_ending_at`]: Self::emit_ending_at
/// [`emit_starting_at`]: Self::emit_starting_at
#[derive(Debug)]
pub struct I2cNarrator {
    scl_line: usize,
    sda_line: usize,
    /// Relative cursor: the instant SCL last went (or will go) low, i.e. where
    /// the next bit period begins.
    cursor: u64,
    /// Edge accumulation and cycle-axis anchoring, shared with every other
    /// narrator (see [`crate::peripherals::wave_plan`]).
    plan: WavePlan,
}

impl I2cNarrator {
    /// A plan for an idle bus (both lines high, as open-drain pull-ups leave
    /// them). `bit_time` is one SCL period in engine cycles; it is clamped to
    /// at least 2 so the low and high half-periods stay distinguishable.
    pub fn new(scl_line: usize, sda_line: usize, bit_time: u64) -> Self {
        let mut idle = vec![true; scl_line.max(sda_line) + 1];
        idle[scl_line] = true;
        idle[sda_line] = true;
        Self {
            scl_line,
            sda_line,
            cursor: 0,
            plan: WavePlan::new(&idle, bit_time),
        }
    }

    /// Half an SCL period — the offset from the start of a bit period to its
    /// rising (sampling) edge.
    #[inline]
    fn half(&self) -> u64 {
        self.plan.bit_time() / 2
    }

    #[inline]
    fn bit_time(&self) -> u64 {
        self.plan.bit_time()
    }

    fn edge(&mut self, line: usize, level: bool, at: u64) {
        self.plan.edge(line, level, at);
    }

    #[inline]
    fn scl(&self) -> bool {
        self.plan.level(self.scl_line)
    }

    #[inline]
    fn sda(&self) -> bool {
        self.plan.level(self.sda_line)
    }

    /// START condition: SDA falls while SCL is high, then SCL falls to open the
    /// first bit period. Occupies one bit time.
    ///
    /// Also serves as a repeated START — it releases SDA high first if the
    /// previous frame left it low, which is exactly the repeated-start setup a
    /// controller drives.
    pub fn start(&mut self) {
        let half = self.half();
        if !self.sda() {
            // Repeated start: release SDA during the low phase, then clock high.
            self.edge(self.sda_line, true, self.cursor);
        }
        if !self.scl() {
            self.edge(self.scl_line, true, self.cursor + half);
            self.cursor += self.bit_time();
        }
        self.edge(self.sda_line, false, self.cursor);
        self.edge(self.scl_line, false, self.cursor + half);
        self.cursor += self.bit_time();
    }

    /// One 9-bit frame: `byte` MSB-first, then the ACK slot (`acked` pulls SDA
    /// low, a NACK leaves it released high).
    ///
    /// Within each bit period SDA is driven at the start, while SCL is low, and
    /// SCL rises at the half-period — so data is stable across every rising
    /// edge, which is the property that makes the trace decodable.
    pub fn frame(&mut self, byte: u8, acked: bool) {
        for bit in (0..8).rev() {
            self.bit_period((byte >> bit) & 1 != 0);
        }
        self.bit_period(!acked);
    }

    /// One SCL period carrying `level` on SDA.
    fn bit_period(&mut self, level: bool) {
        let half = self.half();
        let start = self.cursor;
        self.edge(self.sda_line, level, start);
        self.edge(self.scl_line, true, start + half);
        self.edge(self.scl_line, false, start + self.bit_time());
        self.cursor = start + self.bit_time();
    }

    /// STOP condition: SCL rises, then SDA rises while it is high, leaving the
    /// bus idle.
    pub fn stop(&mut self) {
        let half = self.half();
        // SDA must be low going into a STOP for the rising edge to be visible.
        self.edge(self.sda_line, false, self.cursor);
        self.edge(self.scl_line, true, self.cursor + half);
        self.edge(self.sda_line, true, self.cursor + self.bit_time());
        self.cursor += self.bit_time() + half;
    }

    /// Total span of the plan in engine cycles.
    pub fn span(&self) -> u64 {
        self.plan.span()
    }

    /// `true` when the plan contains no transitions at all.
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty()
    }

    /// Publish the plan so its LAST edge lands on `end_cycle` — the anchoring a
    /// controller that has just finished a phase wants, since it knows when the
    /// phase ended and wants the true bit rate preserved leading up to it.
    ///
    /// The plan needs `span()` cycles of history behind `end_cycle` to occupy.
    /// Real firmware always has it: it boots, initialises clocks and pins, and
    /// only then touches a bus. A transfer fired in the opening cycles of a run
    /// does not, and there is nowhere to put a waveform longer than the run so
    /// far — the capture layer would clamp everything past the present onto a
    /// single cycle, turning a transaction into a spike. Rather than emit that,
    /// the plan is compressed to fit (see [`Self::emit_compressed_into`]): the
    /// shape, the frame count and the bit VALUES all survive, so the trace
    /// still decodes to the bytes that crossed the bus; only the measured bit
    /// rate is no longer the programmed one.
    ///
    /// The returned [`NarrationFit`] says which happened, so a caller that
    /// cares can tell a true-rate trace from a compressed one instead of
    /// silently believing a squashed timebase.
    #[must_use = "a compressed narration does not carry the programmed bit rate"]
    pub fn emit_ending_at(self, lines: &PadLines, end_cycle: u64) -> NarrationFit {
        self.plan.emit_ending_at(lines, end_cycle)
    }

    /// Publish the plan with its first edge at `start_cycle`.
    pub fn emit_starting_at(self, lines: &PadLines, start_cycle: u64) {
        self.plan.emit_starting_at(lines, start_cycle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic_capture::LogicTap;

    const SCL: usize = 0;
    const SDA: usize = 1;
    const LINES: &[&str] = &["SCL", "SDA"];

    /// Capture what an analyzer clipped to both lines would record.
    fn capture(plan: I2cNarrator, start: u64) -> Vec<(u64, u32, bool)> {
        let lines = PadLines::new(LINES, &[true, true]);
        let tap = LogicTap::new();
        // Channel 0 watches SCL, channel 1 watches SDA.
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        plan.emit_starting_at(&lines, start);
        let mut events: Vec<_> = tap
            .take_events()
            .iter()
            .map(|event| (event.cycle, event.ch, event.value))
            .collect();
        events.sort_by_key(|&(cycle, ch, _)| (cycle, ch));
        events
    }

    /// An INDEPENDENT I²C decoder: reconstructs frames from the recorded edges
    /// by protocol rules alone, knowing nothing about how the plan was built.
    /// This is the gate — if the narrated waveform is not decodable, it is
    /// decoration, not a measurement.
    fn decode(events: &[(u64, u32, bool)]) -> Vec<(u8, bool)> {
        // Replay the two lines cycle-ordered, watching for START, sampling SDA
        // on each SCL rising edge, and cutting frames every 9 bits.
        let (mut scl, mut sda) = (true, true);
        let mut started = false;
        let mut bits: Vec<bool> = Vec::new();
        let mut frames = Vec::new();
        for &(_, ch, value) in events {
            let (prev_scl, prev_sda) = (scl, sda);
            if ch == 0 {
                scl = value;
            } else {
                sda = value;
            }
            // START: SDA falls while SCL is high.
            if ch == 1 && prev_sda && !sda && scl {
                started = true;
                bits.clear();
                continue;
            }
            // STOP: SDA rises while SCL is high.
            if ch == 1 && !prev_sda && sda && scl {
                started = false;
                bits.clear();
                continue;
            }
            // Sample on the SCL rising edge.
            if started && ch == 0 && !prev_scl && scl {
                bits.push(sda);
                if bits.len() == 9 {
                    let byte = bits[..8]
                        .iter()
                        .fold(0u8, |acc, &bit| (acc << 1) | u8::from(bit));
                    frames.push((byte, !bits[8]));
                    bits.clear();
                }
            }
        }
        frames
    }

    #[test]
    fn a_narrated_frame_decodes_back_to_the_byte_the_model_exchanged() {
        // The whole point: what the controller transacted is what a decoder
        // reading the pads recovers.
        for byte in [0x00u8, 0x3C, 0xA5, 0xFF, 0x01, 0x80] {
            for acked in [true, false] {
                let mut wave = I2cNarrator::new(SCL, SDA, 80);
                wave.start();
                wave.frame(byte, acked);
                wave.stop();
                let events = capture(wave, 1_000);
                assert_eq!(
                    decode(&events),
                    vec![(byte, acked)],
                    "byte {byte:#04x} ack={acked} did not survive the wire",
                );
            }
        }
    }

    #[test]
    fn a_multi_frame_transaction_decodes_in_order() {
        // Address + two data bytes, the shape of an OLED command write.
        let mut wave = I2cNarrator::new(SCL, SDA, 64);
        wave.start();
        wave.frame(0x3C << 1, true);
        wave.frame(0x00, true);
        wave.frame(0xAF, true);
        wave.stop();
        assert_eq!(
            decode(&capture(wave, 500)),
            vec![(0x78, true), (0x00, true), (0xAF, true)],
        );
    }

    #[test]
    fn scl_runs_at_the_programmed_bit_rate() {
        // A user measuring the bus on the analyzer must read the frequency the
        // controller actually programmed, not one distorted to fit a window.
        const BIT_TIME: u64 = 100;
        let mut wave = I2cNarrator::new(SCL, SDA, BIT_TIME);
        wave.start();
        wave.frame(0x55, true);
        let events = capture(wave, 0);
        let rises: Vec<u64> = events
            .iter()
            .filter(|&&(_, ch, value)| ch == 0 && value)
            .map(|&(cycle, _, _)| cycle)
            .collect();
        assert_eq!(rises.len(), 9, "nine clocks: 8 data bits + ACK");
        for pair in rises.windows(2) {
            assert_eq!(
                pair[1] - pair[0],
                BIT_TIME,
                "SCL period drifted from the programmed bit time",
            );
        }
    }

    #[test]
    fn sda_is_stable_across_every_sampling_edge() {
        // Setup/hold: SDA must not move at the instant SCL rises, or the trace
        // is ambiguous exactly where a decoder reads it.
        let mut wave = I2cNarrator::new(SCL, SDA, 40);
        wave.start();
        wave.frame(0xA5, true);
        wave.stop();
        let events = capture(wave, 0);
        let rises: Vec<u64> = events
            .iter()
            .filter(|&&(_, ch, value)| ch == 0 && value)
            .map(|&(cycle, _, _)| cycle)
            .collect();
        for &(cycle, ch, _) in &events {
            if ch == 1 {
                assert!(
                    !rises.contains(&cycle),
                    "SDA moved on an SCL rising edge at cycle {cycle}",
                );
            }
        }
    }

    #[test]
    fn ending_the_plan_at_a_phase_boundary_preserves_the_bit_rate() {
        const BIT_TIME: u64 = 50;
        const PHASE_END: u64 = 10_000;
        let mut wave = I2cNarrator::new(SCL, SDA, BIT_TIME);
        wave.start();
        wave.frame(0x42, true);
        let span = wave.span();

        let lines = PadLines::new(LINES, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        assert_eq!(wave.emit_ending_at(&lines, PHASE_END), NarrationFit::Exact);
        let events = tap.take_events();

        let last = events.iter().map(|event| event.cycle).max().unwrap();
        assert_eq!(last, PHASE_END, "the plan must land on the phase boundary");
        let first = events.iter().map(|event| event.cycle).min().unwrap();
        assert_eq!(first, PHASE_END - span, "and keep its true duration");
    }

    #[test]
    fn edges_are_published_in_ascending_cycle_order() {
        // The push tap groups adjacent equal-cycle runs; a plan that emitted
        // out of order would be ingested as separate groups.
        let mut wave = I2cNarrator::new(SCL, SDA, 30);
        wave.start();
        wave.frame(0x5A, true);
        wave.stop();
        let lines = PadLines::new(LINES, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        wave.emit_starting_at(&lines, 7);
        let cycles: Vec<u64> = tap.take_events().iter().map(|event| event.cycle).collect();
        assert!(
            cycles.windows(2).all(|pair| pair[0] <= pair[1]),
            "not ascending: {cycles:?}",
        );
    }

    #[test]
    fn a_plan_anchored_near_cycle_zero_saturates_instead_of_underflowing() {
        let mut wave = I2cNarrator::new(SCL, SDA, 64);
        wave.start();
        wave.frame(0xFF, true);
        let lines = PadLines::new(LINES, &[true, true]);
        let _ = wave.emit_ending_at(&lines, 3);
        // Levels still land; the point is that it does not panic.
        assert!(lines.level(SCL) || !lines.level(SCL));
    }

    #[test]
    fn the_bus_is_left_idle_after_a_stop() {
        let mut wave = I2cNarrator::new(SCL, SDA, 64);
        wave.start();
        wave.frame(0x10, true);
        wave.stop();
        let lines = PadLines::new(LINES, &[true, true]);
        wave.emit_starting_at(&lines, 0);
        assert!(lines.level(SCL), "SCL released after STOP");
        assert!(lines.level(SDA), "SDA released after STOP");
    }

    #[test]
    fn a_repeated_start_decodes_as_two_frames() {
        // Write the register pointer, repeated START, then read back — the
        // standard sensor read shape.
        let mut wave = I2cNarrator::new(SCL, SDA, 48);
        wave.start();
        wave.frame(0x53 << 1, true);
        wave.frame(0x32, true);
        wave.start(); // repeated
        wave.frame((0x53 << 1) | 1, true);
        wave.frame(0xE5, false);
        wave.stop();
        assert_eq!(
            decode(&capture(wave, 0)),
            vec![(0xA6, true), (0x32, true), (0xA7, true), (0xE5, false),],
        );
    }

    #[test]
    fn publishing_without_an_armed_tap_still_moves_the_levels() {
        // The analyzer is usually NOT armed; the pad levels must still be right
        // so `read_gpio_pad` and poll-mode sampling see the wire.
        let mut wave = I2cNarrator::new(SCL, SDA, 64);
        wave.start();
        let lines = PadLines::new(LINES, &[true, true]);
        wave.emit_starting_at(&lines, 100);
        assert!(!lines.level(SCL), "START leaves SCL low, mid-frame");
        assert!(!lines.level(SDA));
    }
    #[test]
    fn a_transfer_with_no_history_behind_it_still_decodes() {
        // The limitation this covers: a bus touched in the opening cycles of a
        // run has less elapsed time than its waveform needs. The trace must
        // still say WHAT crossed the bus, even if it cannot say how fast.
        let mut wave = I2cNarrator::new(SCL, SDA, 2_400);
        wave.start();
        wave.frame(0x3C << 1, true);
        wave.frame(0xAF, true);
        wave.stop();
        let span = wave.span();

        const EARLY: u64 = 900; // far less than the ~49k cycles the plan wants
        assert!(span > EARLY, "precondition: the plan does not fit");

        let lines = PadLines::new(LINES, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        let fit = wave.emit_ending_at(&lines, EARLY);

        assert!(
            matches!(fit, NarrationFit::Compressed { .. }),
            "a plan that does not fit must report itself compressed, got {fit:?}",
        );
        let mut events: Vec<_> = tap
            .take_events()
            .iter()
            .map(|event| (event.cycle, event.ch, event.value))
            .collect();
        events.sort_by_key(|&(cycle, ch, _)| (cycle, ch));
        assert_eq!(
            decode(&events),
            vec![(0x78, true), (0xAF, true)],
            "the bytes must survive compression even though the rate does not",
        );
    }

    #[test]
    fn a_compressed_narration_keeps_every_transition_on_its_own_cycle() {
        // Collapsing two transitions onto one cycle would hide an edge from the
        // capture layer's same-cycle last-wins rule — the spike this replaces.
        let mut wave = I2cNarrator::new(SCL, SDA, 4_000);
        wave.start();
        wave.frame(0x55, true);
        wave.stop();

        let lines = PadLines::new(LINES, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        let fit = wave.emit_ending_at(&lines, 40);
        assert!(matches!(fit, NarrationFit::Compressed { .. }));

        let events = tap.take_events();
        let cycles: Vec<u64> = events.iter().map(|e| e.cycle).collect();
        assert!(
            cycles.windows(2).all(|pair| pair[0] <= pair[1]),
            "still ascending after scaling: {cycles:?}",
        );
        // SCL and SDA legitimately move on the same cycle (a bit period ends as
        // the next one's data is set up), and the capture layer keeps both
        // because they are different channels. What must NEVER happen is two
        // transitions of the SAME line sharing a cycle — same-cycle last-wins
        // would silently swallow one, which is how a waveform becomes a spike.
        for channel in [0u32, 1] {
            let per_line: Vec<u64> = events
                .iter()
                .filter(|e| e.ch == channel)
                .map(|e| e.cycle)
                .collect();
            let mut unique = per_line.clone();
            unique.dedup();
            assert_eq!(
                unique.len(),
                per_line.len(),
                "channel {channel} collapsed: {per_line:?}",
            );
        }
        assert!(
            cycles.iter().all(|&c| c <= 40),
            "compression must stay inside the window: {cycles:?}",
        );
    }

    #[test]
    fn a_window_too_small_for_any_timing_emits_no_trace_rather_than_a_wrong_one() {
        // Below one cycle per transition, the capture layer's same-cycle
        // last-wins rule would swallow edges and the trace would decode to
        // bytes that never crossed the bus. Silence is the only honest answer.
        let mut wave = I2cNarrator::new(SCL, SDA, 1_000);
        wave.start();
        wave.frame(0x10, true);
        wave.stop();

        let lines = PadLines::new(LINES, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        let fit = wave.emit_ending_at(&lines, 3);
        assert!(
            matches!(fit, NarrationFit::LevelsOnly { .. }),
            "three cycles cannot carry twenty transitions, got {fit:?}",
        );
        assert!(
            tap.take_events().is_empty(),
            "an unrepresentable waveform must leave no trace to misread",
        );
        assert!(lines.level(SCL), "SCL still released after the STOP");
        assert!(lines.level(SDA), "SDA still released after the STOP");
    }

    #[test]
    fn a_plan_that_fits_is_never_compressed() {
        // Compression must be the exception, not a silent tax on every trace.
        let mut wave = I2cNarrator::new(SCL, SDA, 100);
        wave.start();
        wave.frame(0x42, true);
        wave.stop();
        let lines = PadLines::new(LINES, &[true, true]);
        assert_eq!(wave.emit_ending_at(&lines, 1_000_000), NarrationFit::Exact,);
    }
}
