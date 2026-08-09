//! The ONE way a bus peripheral publishes its wire levels onto GPIO pads.
//!
//! A peripheral that owns a pad in alternate-function / matrix-routed mode is
//! the only thing that knows what the wire is actually doing: the GPIO port's
//! own output register is not driving it. Until that level reaches the pad,
//! `read_gpio_pad` — and therefore the in-engine logic analyzer sampling
//! through it — sees a flat line while the bus is busy. That is the difference
//! between "we decode this bus" and "you can measure this bus".
//!
//! Two peripherals grew their own copy of this mechanism (the generic SPI
//! controller and the ESP32-C3 I²C controller), byte-for-byte parallel apart
//! from how many lines they carry and what those lines are called. This module
//! is that mechanism, once, so the next family to gain bit timing publishes its
//! pads by calling [`PadLines::set`] and gets pad reads, the logic analyzer,
//! and push-mode edge capture with no new plumbing.
//!
//! # What a driver owes this type
//!
//! * Call [`PadLines::set`] (or [`PadLines::set_line`]) at every wire
//!   transition, from the bit engine, on the engine's own clock. Levels are
//!   the state of the WIRE, not of a register: for an open-drain bus that
//!   means the wired-AND of controller and peripheral drive.
//! * Nothing else writes. One writer per line keeps the published waveform
//!   deterministic and lets the reads stay lock-free.
//!
//! Everything past that — pad reads, analyzer sampling, push-mode capture — is
//! handled here and by whichever GPIO model routes the pad.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::logic_capture::LogicTap;

/// Push-capture registration: which analyzer channels currently watch each
/// line, and the tap they report to. Rebuilt by the GPIO model whenever
/// routing changes, so a re-routed pad follows its signal.
#[derive(Debug, Default)]
struct PadTapState {
    tap: Option<LogicTap>,
    /// Watch channels per line, indexed like [`PadLines::names`].
    channels: Vec<Vec<u32>>,
}

/// Live levels of one peripheral's wire, readable by a GPIO model for the pads
/// its routing points here.
///
/// Reads are lock-free (`Relaxed` atomics): a pad read happens on the CPU walk
/// and must not contend with the bit engine. The mutex guards only the tap
/// registration, and is taken solely on a real transition — module-tick or
/// segment-boundary rate, never per engine cycle.
#[derive(Debug)]
pub struct PadLines {
    /// Role names in line order, e.g. `["SCL", "SDA"]` or
    /// `["SCK", "MOSI", "MISO"]`. Static because a peripheral's wire roles are
    /// a property of the silicon, not of a run.
    names: &'static [&'static str],
    levels: Vec<AtomicBool>,
    tap: Mutex<PadTapState>,
}

impl PadLines {
    /// A new wire at its idle levels — the levels a pad reads before the
    /// peripheral has driven anything. `idle` is per line, in `names` order:
    /// idle-high for an open-drain bus with pull-ups, CPOL for SPI's clock.
    ///
    /// # Panics
    /// If `idle.len() != names.len()`. Both are compile-time constants at every
    /// call site, so a mismatch is a bug in the caller, not a runtime condition.
    pub fn new(names: &'static [&'static str], idle: &[bool]) -> Self {
        assert_eq!(
            names.len(),
            idle.len(),
            "pad line idle levels must match line names",
        );
        Self {
            names,
            levels: idle.iter().map(|&level| AtomicBool::new(level)).collect(),
            tap: Mutex::new(PadTapState {
                tap: None,
                channels: vec![Vec::new(); names.len()],
            }),
        }
    }

    /// Role names in line order.
    pub fn names(&self) -> &'static [&'static str] {
        self.names
    }

    /// Line index for a role name, e.g. `"SDA"`. Case-sensitive; the names are
    /// the ones the silicon's datasheet uses.
    pub fn line_index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|&candidate| candidate == name)
    }

    /// Current level of one line. Out-of-range reads as `false` rather than
    /// panicking: a pad read runs on the CPU walk, where a routing table that
    /// has gone stale must not take the engine down.
    pub fn level(&self, line: usize) -> bool {
        self.levels
            .get(line)
            .is_some_and(|level| level.load(Ordering::Relaxed))
    }

    /// Current level of a line by role name.
    pub fn level_of(&self, name: &str) -> Option<bool> {
        self.line_index(name).map(|line| self.level(line))
    }

    /// Drive every line at once — the shape a bit engine that recomputes its
    /// whole wire state per step wants.
    ///
    /// Only lines that actually changed are reported to the tap, so a step that
    /// re-asserts the same levels costs one comparison per line and no lock.
    ///
    /// # Panics
    /// If `levels.len() != names.len()`, for the same reason as [`Self::new`].
    pub fn set(&self, levels: &[bool]) {
        assert_eq!(
            levels.len(),
            self.levels.len(),
            "pad line level count must match line names",
        );
        let mut changed: Option<Vec<(usize, bool)>> = None;
        for (line, &next) in levels.iter().enumerate() {
            let previous = self.levels[line].swap(next, Ordering::Relaxed);
            if previous != next {
                changed.get_or_insert_with(Vec::new).push((line, next));
            }
        }
        if let Some(changed) = changed {
            self.report(&changed);
        }
    }

    /// Drive a single line — the shape an engine that toggles one wire at a
    /// time (a clock edge, a data setup) wants.
    pub fn set_line(&self, line: usize, level: bool) {
        let Some(cell) = self.levels.get(line) else {
            return;
        };
        if cell.swap(level, Ordering::Relaxed) != level {
            self.report(&[(line, level)]);
        }
    }

    /// Drive one line at a known past cycle — the shape a transaction-level
    /// controller narrating a completed phase wants (see
    /// [`crate::peripherals::i2c_waveform`]).
    ///
    /// Identical to [`set_line`](Self::set_line) except the reported edge
    /// carries `cycle` instead of the tap's provisional clock, so nine SCL
    /// periods' worth of edges land spread across the cycles they occupied
    /// rather than piled onto the cycle the phase retired at. Callers emit a
    /// run in ascending cycle order.
    pub fn set_line_at(&self, line: usize, level: bool, cycle: u64) {
        let Some(cell) = self.levels.get(line) else {
            return;
        };
        if cell.swap(level, Ordering::Relaxed) != level {
            self.report_at(&[(line, level)], Some(cycle));
        }
    }

    /// Report real transitions to any armed push-capture channels. Split out so
    /// the lock is touched only when a line moved.
    fn report(&self, changed: &[(usize, bool)]) {
        self.report_at(changed, None);
    }

    /// `at = None` stamps with the tap's provisional clock (the bit-engine
    /// path); `at = Some(cycle)` stamps explicitly (the narrated path).
    fn report_at(&self, changed: &[(usize, bool)], at: Option<u64>) {
        let state = self.tap.lock().unwrap();
        let Some(tap) = &state.tap else {
            return;
        };
        for &(line, level) in changed {
            let Some(channels) = state.channels.get(line) else {
                continue;
            };
            for &channel in channels {
                match at {
                    Some(cycle) => tap.push_at(channel, level, cycle),
                    None => tap.push(channel, level),
                }
            }
        }
    }

    /// Install (or clear, with `tap = None`) the push-capture registration.
    /// `channels` is per line, in `names` order. The GPIO model calls this when
    /// its routing changes, so a pad that stops being routed here stops
    /// reporting.
    pub fn install_tap(&self, tap: Option<LogicTap>, channels: Vec<Vec<u32>>) {
        let mut state = self.tap.lock().unwrap();
        state.tap = tap;
        state.channels = channels;
        state.channels.resize(self.levels.len(), Vec::new());
    }

    /// Clear the registration and every watch channel — the disarm path.
    pub fn clear_tap(&self) {
        self.install_tap(None, Vec::new());
    }

    /// The installed tap's provisional "now", or `None` when nothing is
    /// capturing this wire.
    ///
    /// A bit engine never needs this — it drives as the engine advances. A
    /// transaction-level controller narrating a phase it has just finished
    /// reads it to know which cycle to anchor the narration's last edge to.
    pub fn tap_clock(&self) -> Option<u64> {
        let state = self.tap.lock().unwrap();
        state.tap.as_ref().map(|tap| tap.clock())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const I2C: &[&str] = &["SCL", "SDA"];

    #[test]
    fn starts_at_the_idle_levels_the_wire_actually_rests_at() {
        // Open-drain with pull-ups idles high; a pad read before the controller
        // has driven anything must say high, not "false because unset".
        let lines = PadLines::new(I2C, &[true, true]);
        assert!(lines.level_of("SCL").unwrap());
        assert!(lines.level_of("SDA").unwrap());
    }

    #[test]
    fn resolves_lines_by_the_datasheet_role_name() {
        let lines = PadLines::new(I2C, &[true, true]);
        assert_eq!(lines.line_index("SDA"), Some(1));
        assert_eq!(lines.line_index("MOSI"), None);
        assert_eq!(lines.level_of("MOSI"), None);
    }

    #[test]
    fn reports_only_lines_that_actually_moved() {
        let lines = PadLines::new(I2C, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);

        lines.set(&[false, true]); // SCL falls, SDA holds
        lines.set(&[false, true]); // nothing moves
        lines.set(&[false, false]); // SDA falls

        let events = tap.take_events();
        assert_eq!(
            events
                .iter()
                .map(|event| (event.ch, event.value))
                .collect::<Vec<_>>(),
            vec![(0, false), (1, false)],
            "a re-asserted level is not an edge",
        );
    }

    #[test]
    fn one_line_can_be_driven_without_restating_the_others() {
        let lines = PadLines::new(I2C, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![7], vec![8]]);

        lines.set_line(0, false);
        assert!(!lines.level(0));
        assert!(lines.level(1), "the untouched line holds its level");
        assert_eq!(
            tap.take_events()
                .iter()
                .map(|event| (event.ch, event.value))
                .collect::<Vec<_>>(),
            vec![(7, false)],
        );
    }

    #[test]
    fn a_pad_that_stops_being_routed_here_stops_reporting() {
        let lines = PadLines::new(I2C, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        lines.set(&[false, false]);
        assert_eq!(tap.take_events().len(), 2);

        lines.clear_tap();
        lines.set(&[true, true]);
        assert!(
            tap.take_events().is_empty(),
            "levels still move, but nothing is watching this wire",
        );
        assert!(lines.level(0), "clearing the tap does not stop the wire");
    }

    #[test]
    fn levels_stay_readable_when_the_routing_table_is_stale() {
        // A pad read runs on the CPU walk; a line index that no longer exists
        // must read as low, not panic the engine.
        let lines = PadLines::new(I2C, &[true, true]);
        assert!(!lines.level(9));
        lines.set_line(9, true);
    }

    #[test]
    fn a_short_channel_list_still_covers_every_line() {
        let lines = PadLines::new(I2C, &[true, true]);
        let tap = LogicTap::new();
        // Caller registered only the first line; the second must not index
        // out of bounds when it moves.
        lines.install_tap(Some(tap.clone()), vec![vec![3]]);
        lines.set(&[false, false]);
        assert_eq!(
            tap.take_events()
                .iter()
                .map(|event| event.ch)
                .collect::<Vec<_>>(),
            vec![3],
        );
    }
}
