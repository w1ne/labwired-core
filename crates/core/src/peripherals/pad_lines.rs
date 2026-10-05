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

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;

use crate::logic_capture::{LogicTap, PadDrive};

/// How a peripheral's output stage drives one of its lines.
///
/// Every line starts [`PushPull`](LineDrive::PushPull), which is what every
/// publisher here assumed before lines had a drive: the peripheral owns the
/// pad and the level it publishes is the level the pad carries. A peripheral
/// that shares a wire with other chips (a pad on a world `gpio_net`) says
/// what its output stage really does, so the net can resolve the wire:
///
/// * [`OpenDrain`](LineDrive::OpenDrain): the published level `false` pulls
///   the wire low; `true` releases it (I²C SCL/SDA).
/// * [`Input`](LineDrive::Input): the line is an input for this peripheral
///   right now (an SPI master's MISO, a deselected slave's MISO, a slave's
///   SCK/MOSI/NSS) and drives nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineDrive {
    PushPull,
    OpenDrain,
    Input,
}

impl LineDrive {
    fn code(self) -> u8 {
        match self {
            Self::PushPull => 0,
            Self::OpenDrain => 1,
            Self::Input => 2,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::OpenDrain,
            2 => Self::Input,
            _ => Self::PushPull,
        }
    }
}

const NO_NET_IDLE: u8 = u8::MAX;
const NO_STAMP: u64 = u64::MAX;

/// A pad level an outside driver applied to a line's pad (through the GPIO
/// port's `set_gpio_input`), for the peripheral that owns the line to read.
/// Collected by the GPIO port, delivered by the bus to the owner through
/// [`crate::Peripheral::wire_input_edge`].
#[derive(Debug, Clone)]
pub struct WireInputEdge {
    pub cell: std::sync::Arc<PadLines>,
    pub line: usize,
    pub level: bool,
}

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
    /// Per line: [`LineDrive`] code. All push-pull until a peripheral says
    /// otherwise.
    modes: Vec<AtomicU8>,
    /// Per line: the level an outside driver holds on the routed pad, 0 when
    /// none has (unknown), 1 low, 2 high. Written by the GPIO port only.
    inputs: Vec<AtomicU8>,
    /// Per line: a pad that can carry this line sits on a world `gpio_net`.
    on_net: Vec<AtomicBool>,
    /// Per line: the [`LineDrive`] code the line takes when it is put on a
    /// net, before its peripheral has said anything; `NO_NET_IDLE` keeps it.
    net_idle: Vec<AtomicU8>,
    /// Engine cycle to stamp reports with while the owner processes an event
    /// it knows the time of; `NO_STAMP` uses the tap's provisional clock.
    stamp: AtomicU64,
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
            modes: idle.iter().map(|_| AtomicU8::new(0)).collect(),
            inputs: idle.iter().map(|_| AtomicU8::new(0)).collect(),
            on_net: idle.iter().map(|_| AtomicBool::new(false)).collect(),
            net_idle: idle.iter().map(|_| AtomicU8::new(NO_NET_IDLE)).collect(),
            stamp: AtomicU64::new(NO_STAMP),
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

    /// How this peripheral drives `line` now. Out of range reads push-pull.
    pub fn mode(&self, line: usize) -> LineDrive {
        self.modes.get(line).map_or(LineDrive::PushPull, |m| {
            LineDrive::from_code(m.load(Ordering::Relaxed))
        })
    }

    /// Change how this peripheral drives `line`. A change is reported to the
    /// push-capture channels with the line's new drive, so a pad's four-state
    /// trace (and a world net reading it) sees a release or a takeover the
    /// moment it happens.
    pub fn set_mode(&self, line: usize, mode: LineDrive) {
        let Some(cell) = self.modes.get(line) else {
            return;
        };
        if cell.swap(mode.code(), Ordering::Relaxed) != mode.code() {
            self.report_with(&[(line, self.level(line))], None, true);
        }
    }

    /// Set the published output level and the drive of one line together,
    /// reporting once if either moved.
    pub fn drive_line(&self, line: usize, mode: LineDrive, level: bool) {
        let (Some(m), Some(l)) = (self.modes.get(line), self.levels.get(line)) else {
            return;
        };
        let mode_moved = m.swap(mode.code(), Ordering::Relaxed) != mode.code();
        let level_moved = l.swap(level, Ordering::Relaxed) != level;
        if mode_moved || level_moved {
            self.report_with(&[(line, level)], None, mode_moved);
        }
    }

    /// What this peripheral's output stage does to the pad carrying `line`:
    /// driven, or released / an input (high-Z).
    pub fn pad_drive(&self, line: usize) -> PadDrive {
        match self.mode(line) {
            LineDrive::PushPull => PadDrive::Driven,
            LineDrive::OpenDrain if !self.level(line) => PadDrive::Driven,
            LineDrive::OpenDrain | LineDrive::Input => PadDrive::HighZ,
        }
    }

    /// `true` while this peripheral's own output stage drives the pad.
    pub fn drives(&self, line: usize) -> bool {
        self.pad_drive(line) == PadDrive::Driven
    }

    /// The level an outside driver holds on the pad routed to `line`, or
    /// `None` when nothing outside has driven it.
    pub fn input(&self, line: usize) -> Option<bool> {
        match self.inputs.get(line)?.load(Ordering::Relaxed) {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }

    /// Record the level an outside driver holds on the pad routed to `line`.
    /// Returns `true` when it changed. GPIO-port side only.
    pub fn set_input(&self, line: usize, level: bool) -> bool {
        let Some(cell) = self.inputs.get(line) else {
            return false;
        };
        let code = if level { 2 } else { 1 };
        cell.swap(code, Ordering::Relaxed) != code
    }

    /// The level the WIRE carries as this peripheral sees it at its pad:
    /// the outside level when one is known, else its own output.
    pub fn wire_level(&self, line: usize) -> bool {
        self.input(line).unwrap_or_else(|| self.level(line))
    }

    /// Stamp every report from now on with engine cycle `cycle` (`None`:
    /// back to the tap's provisional clock). A bit engine processing a
    /// scheduler event or an input edge at a known cycle sets it, so its
    /// edges land at that cycle even when the event is drained late; it
    /// clears it when done.
    pub fn set_stamp(&self, cycle: Option<u64>) {
        self.stamp
            .store(cycle.unwrap_or(NO_STAMP), Ordering::Relaxed);
    }

    fn stamp(&self) -> Option<u64> {
        let cycle = self.stamp.load(Ordering::Relaxed);
        (cycle != NO_STAMP).then_some(cycle)
    }

    /// Mark `line` as reachable from a pad on a world `gpio_net`. Set once
    /// at world build, when the net isolates its pads. A line whose
    /// peripheral declared a net idle drive ([`Self::set_net_idle`]) takes it
    /// now, so the pad does not drive the shared wire before firmware has
    /// configured anything.
    pub fn mark_on_net(&self, line: usize) {
        if let Some(flag) = self.on_net.get(line) {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(code) = self.net_idle.get(line).map(|c| c.load(Ordering::Relaxed)) {
            if code != NO_NET_IDLE {
                self.set_mode(line, LineDrive::from_code(code));
            }
        }
    }

    /// Declare the drive `line` takes when it is put on a net, until the
    /// peripheral states one. Peripherals that know nets declare this when
    /// they create the cell; every other publisher keeps push-pull.
    pub fn set_net_idle(&self, line: usize, mode: LineDrive) {
        if let Some(code) = self.net_idle.get(line) {
            code.store(mode.code(), Ordering::Relaxed);
        }
    }

    /// `true` when a pad that can carry `line` sits on a world `gpio_net`.
    pub fn on_net(&self, line: usize) -> bool {
        self.on_net
            .get(line)
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// `true` when any line of this wire is on a world `gpio_net`.
    pub fn any_on_net(&self) -> bool {
        self.on_net.iter().any(|flag| flag.load(Ordering::Relaxed))
    }

    /// Report real transitions to any armed push-capture channels. Split out so
    /// the lock is touched only when a line moved.
    fn report(&self, changed: &[(usize, bool)]) {
        self.report_at(changed, None);
    }

    /// `at = None` stamps with the tap's provisional clock (the bit-engine
    /// path); `at = Some(cycle)` stamps explicitly (the narrated path).
    fn report_at(&self, changed: &[(usize, bool)], at: Option<u64>) {
        self.report_with(changed, at, false);
    }

    /// `with_drive` reports the drive even on a push-pull line (a mode
    /// change into push-pull is a drive change too).
    fn report_with(&self, changed: &[(usize, bool)], at: Option<u64>, with_drive: bool) {
        let at = at.or_else(|| self.stamp());
        let state = self.tap.lock().unwrap();
        let Some(tap) = &state.tap else {
            return;
        };
        for &(line, level) in changed {
            let Some(channels) = state.channels.get(line) else {
                continue;
            };
            // A push-pull line keeps the drive the GPIO port seeded (the
            // pre-drive behaviour, unchanged); any other mode says what the
            // output stage does, and a line that does not drive the pad shows
            // the level the wire holds there.
            let mode = self.mode(line);
            let (value, drive) = match mode {
                LineDrive::PushPull if !with_drive => (level, None),
                LineDrive::PushPull => (level, Some(PadDrive::Driven)),
                _ => {
                    let drive = self.pad_drive(line);
                    let value = if drive == PadDrive::Driven {
                        level
                    } else {
                        self.input(line).unwrap_or(level)
                    };
                    (value, Some(drive))
                }
            };
            for &channel in channels {
                tap.push_event(channel, value, drive, at);
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

    /// Replace only ONE routing table's channel registrations on this wire,
    /// leaving every other table's alone.
    ///
    /// # Why this is not [`install_tap`]
    ///
    /// One wire reaches pads on SEVERAL GPIO ports, and each port owns its own
    /// [`PadRoutes`](super::pad_routing::PadRoutes) holding a handle to this
    /// same cell. `install_tap` replaces the whole channel table, so the LAST
    /// port to sync silently erased every earlier port's channels.
    ///
    /// That is invisible in the common case, because most labs put a bus's
    /// pads on ONE port — an STM32H563 clips SPI1 to PA5/PA7 and both
    /// registrations come from `gpioa`. It becomes unavoidable on a part whose
    /// pads are SPLIT across ports: the STM32WBA52 has exactly one SPI and its
    /// datasheet puts `SPI1_SCK` on PB4 while `SPI1_MOSI` is on PA15
    /// (DS14127 Rev 10 Table 25, pages 76-77). Watching both, `gpioa`
    /// registered MOSI and then `gpiob` overwrote the table with SCK alone —
    /// the clock captured, the data channel silently empty, and the pad still
    /// READING correctly the whole time, so levels looked right and only the
    /// trace was wrong.
    ///
    /// [`clear_taps`](super::pad_routing::PadRoutes::clear_taps) already
    /// documents this hazard for the DISARM path; this is the same hazard on
    /// the arm path.
    ///
    /// `remove` is what the calling table registered last time, `add` is what
    /// it wants now; both are indexed by line like [`PadLines::names`]. The tap
    /// is dropped only once NO line has a watcher left.
    pub fn merge_tap(&self, tap: Option<LogicTap>, remove: &[Vec<u32>], add: &[Vec<u32>]) {
        let mut state = self.tap.lock().unwrap();
        state.channels.resize(self.levels.len(), Vec::new());
        for (line, channels) in state.channels.iter_mut().enumerate() {
            if let Some(stale) = remove.get(line) {
                channels.retain(|c| !stale.contains(c));
            }
            if let Some(fresh) = add.get(line) {
                for &channel in fresh {
                    if !channels.contains(&channel) {
                        channels.push(channel);
                    }
                }
            }
        }
        let any = state.channels.iter().any(|c| !c.is_empty());
        state.tap = if any { tap } else { None };
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
    fn an_open_drain_line_drives_only_its_zero_and_reports_the_release() {
        let lines = PadLines::new(I2C, &[true, true]);
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![1]]);
        lines.set_mode(1, LineDrive::OpenDrain);
        assert!(!lines.drives(1), "released while high");
        lines.drive_line(1, LineDrive::OpenDrain, false);
        assert!(lines.drives(1));
        // Released while the wire is still held low elsewhere: the pad shows
        // the wire, not this chip's output.
        lines.set_input(1, false);
        lines.drive_line(1, LineDrive::OpenDrain, true);
        let events: Vec<_> = tap
            .take_events()
            .iter()
            .map(|e| (e.ch, e.value, e.drive))
            .collect();
        assert_eq!(
            events,
            vec![
                (1, true, Some(PadDrive::HighZ)),
                (1, false, Some(PadDrive::Driven)),
                (1, false, Some(PadDrive::HighZ)),
            ]
        );
        assert!(!lines.wire_level(1));
    }

    #[test]
    fn a_line_put_on_a_net_takes_the_drive_its_owner_declared() {
        let lines = PadLines::new(I2C, &[true, true]);
        lines.set_net_idle(0, LineDrive::Input);
        assert_eq!(lines.mode(0), LineDrive::PushPull);
        lines.mark_on_net(0);
        lines.mark_on_net(1);
        assert_eq!(lines.mode(0), LineDrive::Input);
        assert_eq!(
            lines.mode(1),
            LineDrive::PushPull,
            "no declaration: unchanged"
        );
        assert!(lines.on_net(0) && lines.any_on_net());
        assert_eq!(lines.input(0), None);
        assert!(lines.set_input(0, true));
        assert!(!lines.set_input(0, true), "same level: no edge");
        assert_eq!(lines.input(0), Some(true));
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
