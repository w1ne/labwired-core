// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The ONE way a narrated waveform is placed on the engine's cycle axis.
//!
//! A narrator knows a *protocol*: I²C says "START, nine-bit frames, STOP",
//! UART says "start bit, data LSB-first, parity, stop bits". None of that has
//! anything to do with *when* the edges land, which is the same problem every
//! time:
//!
//! * the transitions must be stamped at the cycles they occupied, not piled
//!   onto the instant the transfer retired;
//! * they must reach far enough back to fit, and a young run may not have that
//!   much history;
//! * two transitions of ONE line must never share a cycle, because the capture
//!   layer keeps a single level per channel per cycle and would swallow one —
//!   turning a transaction into a spike that decodes to bytes which never
//!   crossed the bus.
//!
//! [`WavePlan`] owns exactly that. A narrator builds edges in relative time and
//! hands the finished plan a cycle to end at; the plan decides whether it fits
//! at its true rate, has to be compressed, or cannot be represented at all, and
//! says which via [`NarrationFit`].
//!
//! Splitting it this way is what lets a new protocol be a new narrator rather
//! than a second copy of the anchoring rules.

use super::pad_lines::PadLines;

/// How well a narration fitted the cycles available to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NarrationFit {
    /// The whole waveform fitted at its programmed rate — measuring the trace
    /// gives the frequency the controller's registers ask for.
    Exact,
    /// The run was younger than the waveform, so it was scaled to fit. Bit
    /// values, framing and flags are intact and the trace decodes correctly;
    /// the timebase is not the programmed one.
    Compressed {
        /// Cycles per bit the controller actually programmed.
        programmed: u64,
        /// Cycles the compressed waveform was squeezed into.
        occupied: u64,
    },
    /// Fewer cycles had elapsed than the waveform has transitions, so no
    /// timeline can hold it. Only the net levels are applied — pad reads stay
    /// correct and the trace stays empty rather than wrong.
    LevelsOnly {
        /// Transitions the waveform contains.
        transitions: u64,
        /// Cycles that had actually elapsed to hold them.
        available: u64,
    },
}

/// `value * numerator / denominator` without overflowing on realistic spans.
fn mul_div(value: u64, numerator: u64, denominator: u64) -> u64 {
    ((value as u128 * numerator as u128) / denominator as u128) as u64
}

/// One planned transition, in cycles relative to the start of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannedEdge {
    at: u64,
    line: usize,
    level: bool,
}

/// A protocol-agnostic waveform under construction.
///
/// Times are relative until [`emit_ending_at`](Self::emit_ending_at) anchors
/// them to engine cycles.
#[derive(Debug)]
pub struct WavePlan {
    /// Current level of each line, so re-asserting a level is not an edge.
    levels: Vec<bool>,
    /// Cycles per bit — carried so a compressed fit can report what rate was
    /// programmed.
    bit_time: u64,
    edges: Vec<PlannedEdge>,
}

impl WavePlan {
    /// A plan for a wire resting at `idle`. `bit_time` is one bit period in
    /// engine cycles, clamped to at least 2 so a bit's halves stay distinct.
    pub fn new(idle: &[bool], bit_time: u64) -> Self {
        Self {
            levels: idle.to_vec(),
            bit_time: bit_time.max(2),
            edges: Vec::new(),
        }
    }

    /// One bit period in engine cycles.
    #[inline]
    pub fn bit_time(&self) -> u64 {
        self.bit_time
    }

    /// Drive `line` to `level` at relative cycle `at`. Re-asserting the level a
    /// line already holds records nothing.
    pub fn edge(&mut self, line: usize, level: bool, at: u64) {
        let Some(current) = self.levels.get_mut(line) else {
            return;
        };
        if *current == level {
            return;
        }
        *current = level;
        self.edges.push(PlannedEdge { at, line, level });
    }

    /// The level `line` currently holds in the plan.
    pub fn level(&self, line: usize) -> bool {
        self.levels.get(line).copied().unwrap_or(false)
    }

    /// Total span of the plan in engine cycles.
    pub fn span(&self) -> u64 {
        self.edges.last().map_or(0, |edge| edge.at)
    }

    /// `true` when the plan contains no transitions at all.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Publish so the LAST edge lands on `end_cycle`, preserving the true rate
    /// when the run is old enough to hold the waveform, and compressing to fit
    /// when it is not. See [`NarrationFit`].
    #[must_use = "a compressed narration does not carry the programmed rate"]
    pub fn emit_ending_at(self, lines: &PadLines, end_cycle: u64) -> NarrationFit {
        self.emit_between(lines, 0, end_cycle)
    }

    /// Publish ending at `end_cycle` while reaching no further back than
    /// `not_before`.
    ///
    /// A bus that falls silent between transactions can reach back as far as it
    /// likes, which is what [`emit_ending_at`] does. A line that narrates
    /// repeatedly — a UART flushing one burst of characters after another —
    /// cannot: reaching back over cycles an earlier flush already painted would
    /// re-drive levels the capture layer has already recorded, inventing
    /// transitions that never happened. Passing the previous flush's end as
    /// `not_before` confines each narration to cycles no other narration owns.
    ///
    /// [`emit_ending_at`]: Self::emit_ending_at
    #[must_use = "a compressed narration does not carry the programmed rate"]
    pub fn emit_between(self, lines: &PadLines, not_before: u64, end_cycle: u64) -> NarrationFit {
        let span = self.span();
        let available = end_cycle.saturating_sub(not_before);
        if span <= available {
            self.emit_starting_at(lines, end_cycle - span);
            NarrationFit::Exact
        } else {
            self.emit_compressed_into(lines, available, end_cycle)
        }
    }

    /// Publish with the first edge at `start_cycle`, in ascending cycle order
    /// as the push tap expects.
    pub fn emit_starting_at(self, lines: &PadLines, start_cycle: u64) {
        for edge in &self.edges {
            lines.set_line_at(edge.line, edge.level, start_cycle + edge.at);
        }
    }

    /// Scale the plan into the `available` cycles ending at `end_cycle`.
    fn emit_compressed_into(
        self,
        lines: &PadLines,
        available: u64,
        end_cycle: u64,
    ) -> NarrationFit {
        let span = self.span();
        let instants = self.distinct_instants();
        if span == 0 || available < instants {
            // Below one cycle per transition there is no honest rendering: the
            // capture layer keeps a single level per channel per cycle, so any
            // packing collapses transitions and the trace would decode to bytes
            // that never crossed the bus. Apply where each line ENDED and emit
            // no trace — silence beats a confident wrong answer.
            let mut final_level: Vec<Option<bool>> = vec![None; lines.names().len()];
            for edge in &self.edges {
                if let Some(slot) = final_level.get_mut(edge.line) {
                    *slot = Some(edge.level);
                }
            }
            for (line, level) in final_level.iter().enumerate() {
                if let Some(level) = level {
                    lines.set_line(line, *level);
                }
            }
            return NarrationFit::LevelsOnly {
                transitions: instants,
                available,
            };
        }
        // Keep edge order, keep simultaneous edges simultaneous, and keep every
        // transition of one line on its own cycle so none is swallowed.
        let floor = end_cycle - instants;
        let mut previous_at = None;
        let mut previous_cycle = floor;
        for edge in &self.edges {
            let cycle = if previous_at == Some(edge.at) {
                previous_cycle
            } else {
                let scaled = floor + mul_div(edge.at, instants, span);
                let next = scaled.max(previous_cycle.saturating_add(1));
                previous_at = Some(edge.at);
                previous_cycle = next;
                next
            };
            lines.set_line_at(edge.line, edge.level, cycle);
        }
        NarrationFit::Compressed {
            programmed: self.bit_time,
            occupied: end_cycle.saturating_sub(floor),
        }
    }

    /// Distinct instants in the plan — the minimum cycles a compressed
    /// narration needs so no two transitions share one.
    fn distinct_instants(&self) -> u64 {
        let mut count = 0;
        let mut previous = None;
        for edge in &self.edges {
            if previous != Some(edge.at) {
                count += 1;
                previous = Some(edge.at);
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic_capture::LogicTap;

    const ONE: &[&str] = &["TX"];

    fn wire() -> PadLines {
        PadLines::new(ONE, &[true])
    }

    #[test]
    fn re_asserting_a_level_is_not_an_edge() {
        let mut plan = WavePlan::new(&[true], 10);
        plan.edge(0, true, 0); // already high
        plan.edge(0, false, 10);
        plan.edge(0, false, 20); // already low
        assert_eq!(plan.edges.len(), 1);
        assert!(!plan.level(0));
    }

    #[test]
    fn a_plan_that_fits_keeps_its_true_spacing() {
        let mut plan = WavePlan::new(&[true], 100);
        plan.edge(0, false, 100);
        plan.edge(0, true, 200);
        let lines = wire();
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0]]);
        assert_eq!(plan.emit_ending_at(&lines, 10_000), NarrationFit::Exact);
        let cycles: Vec<u64> = tap.take_events().iter().map(|e| e.cycle).collect();
        assert_eq!(cycles, vec![9_900, 10_000]);
    }

    #[test]
    fn a_plan_too_long_for_the_run_compresses_but_keeps_every_transition() {
        let mut plan = WavePlan::new(&[true], 1_000);
        // Alternate starting LOW: starting high would re-assert the idle level,
        // which is correctly not an edge at all.
        for i in 0..10u64 {
            plan.edge(0, i % 2 == 1, (i + 1) * 1_000);
        }
        let lines = wire();
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0]]);
        let fit = plan.emit_ending_at(&lines, 50);
        assert!(matches!(fit, NarrationFit::Compressed { .. }), "{fit:?}");
        let cycles: Vec<u64> = tap.take_events().iter().map(|e| e.cycle).collect();
        assert_eq!(cycles.len(), 10, "no transition dropped");
        let mut unique = cycles.clone();
        unique.dedup();
        assert_eq!(unique.len(), cycles.len(), "none collapsed: {cycles:?}");
        assert!(cycles.iter().all(|&c| c <= 50));
    }

    #[test]
    fn a_run_too_young_for_any_timing_emits_levels_only() {
        // The plan must END LOW, i.e. NOT at the idle level. Ending high would
        // make `set_line` a no-op and the two assertions below would hold on a
        // `PadLines` that was never touched — the whole level-application block
        // could be deleted and this test would stay green.
        let mut plan = WavePlan::new(&[true], 1_000);
        for i in 0..11u64 {
            plan.edge(0, i % 2 == 1, (i + 1) * 1_000);
        }
        let lines = wire();
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0]]);
        let fit = plan.emit_ending_at(&lines, 3);
        assert!(matches!(fit, NarrationFit::LevelsOnly { .. }), "{fit:?}");
        // The NET level change is still reported, and that is honest — the wire
        // really did end up there. What must not appear is a waveform: ten
        // transitions squeezed into three cycles would decode to bytes that
        // never crossed the wire.
        let events = tap.take_events();
        assert_eq!(
            events.len(),
            1,
            "exactly the net level, never a fabricated waveform: {events:?}",
        );
        assert!(!events[0].value, "and it reports where the wire ended up");
        // i = 10 is the last edge and drives LOW, so that is where the wire
        // must rest — pad reads stay correct even when the trace cannot exist,
        // and the level genuinely had to move to get there.
        assert!(
            !lines.level(0),
            "the line still ends where the plan left it"
        );
    }

    #[test]
    fn simultaneous_edges_on_different_lines_stay_simultaneous() {
        const TWO: &[&str] = &["A", "B"];
        let mut plan = WavePlan::new(&[true, true], 10);
        plan.edge(0, false, 100);
        plan.edge(1, false, 100);
        let lines = PadLines::new(TWO, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        let _ = plan.emit_ending_at(&lines, 100);
        let cycles: Vec<u64> = tap.take_events().iter().map(|e| e.cycle).collect();
        assert_eq!(cycles, vec![100, 100], "one instant, two channels");
    }

    #[test]
    fn a_floor_stops_a_narration_repainting_cycles_an_earlier_one_owns() {
        // The previous flush painted up to cycle 1000. This plan needs 100
        // cycles but retires only 5 later, so reaching back its full span would
        // re-drive levels already recorded there. The floor forces it to
        // compress into the cycles it actually owns.
        let mut plan = WavePlan::new(&[true], 50);
        plan.edge(0, false, 50);
        plan.edge(0, true, 100);
        let lines = wire();
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0]]);
        let fit = plan.emit_between(&lines, 1_000, 1_005);
        assert!(matches!(fit, NarrationFit::Compressed { .. }), "{fit:?}");
        let cycles: Vec<u64> = tap.take_events().iter().map(|e| e.cycle).collect();
        assert!(
            cycles.iter().all(|&c| c > 1_000),
            "nothing reaches back into cycles the previous narration owns: {cycles:?}",
        );
    }

    #[test]
    fn a_floor_that_leaves_room_costs_no_fidelity() {
        let mut plan = WavePlan::new(&[true], 50);
        plan.edge(0, false, 50);
        plan.edge(0, true, 100);
        let lines = wire();
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0]]);
        assert_eq!(plan.emit_between(&lines, 1_000, 2_000), NarrationFit::Exact);
        let cycles: Vec<u64> = tap.take_events().iter().map(|e| e.cycle).collect();
        assert_eq!(cycles, vec![1_950, 2_000]);
    }

    #[test]
    fn an_empty_plan_publishes_nothing() {
        let plan = WavePlan::new(&[true], 10);
        assert!(plan.is_empty());
        assert_eq!(plan.span(), 0);
    }
}
