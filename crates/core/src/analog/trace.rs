// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The bounded analog sample ring the oscilloscope instrument reads.
//!
//! One row per co-simulation step per model, holding every probed value plus
//! any extra `trace:` expressions. Cursors are sample sequence numbers, the
//! same contract as [`crate::Machine::logic_read_edges`]: a reader passes back
//! the `next_cursor` it was given and receives only what is newer.
//!
//! ## One ring, several models
//!
//! All analog models registered on one [`crate::cosim::CosimRunner`] share a
//! single ring, each owning a contiguous block of channels. A model that steps
//! writes a full row: its own columns get the values it just solved, and every
//! other model's columns are carried forward from their last sample. That is
//! what a scope shows — a channel holds its level between updates — and it is
//! the only way one `u64` cursor can index samples from models running at
//! different `step_ns`.
//! During graph initialization, multiple models can publish their initial
//! values at 0 ns without advancing time. Those writes merge into one resolved
//! initial row so an uninitialized channel is not displayed as a zero sample.
//!
//! ## Overflow
//!
//! The ring holds `trace_samples` rows (default [`DEFAULT_TRACE_SAMPLES`], two
//! seconds at a 100 µs step). Past that the OLDEST row is dropped and counted,
//! so a slow reader loses history, never the present.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Default ring depth: 20 000 samples, two seconds at a 100 µs co-sim step.
pub const DEFAULT_TRACE_SAMPLES: usize = 20_000;

/// Most change points one channel of [`AnalogSeries`] keeps for a run.
/// Past this the channel stops recording and counts what it dropped, so a
/// reader can tell a truncated series from a quiet one.
pub const DEFAULT_SERIES_POINTS: usize = 65_536;

/// Absolute deadband of the run-long series: 10 uA for a current channel.
pub const SERIES_DEADBAND_AMPS: f64 = 1e-5;
/// Absolute deadband of the run-long series: 1 mV for a voltage channel.
pub const SERIES_DEADBAND_VOLTS: f64 = 1e-3;
/// Absolute deadband of the run-long series: 1 rpm for a motor speed channel.
pub const SERIES_DEADBAND_RPM: f64 = 1.0;
/// Relative deadband: a change smaller than 0.5 % of the held value is not a
/// new point.
pub const SERIES_DEADBAND_REL: f64 = 5e-3;

/// One channel of the run-long, change-point series.
///
/// Sample-and-hold: each `[time_ns, value]` point holds until the next one.
/// A point is written when the value moves by more than the channel's
/// deadband from the last written point, so the series covers the WHOLE run
/// (unlike the 2 s scope ring) at a size set by how often the circuit changes,
/// not by how long it runs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnalogSeriesChannel {
    /// Same name as the scope channel (`<model>.<probe or trace expression>`).
    pub name: String,
    /// Engineering unit such as `"V"`, `"mV"`, `"A"`, or `"rpm"`.
    pub unit: String,
    /// `[time_ns, value]` change points, oldest first.
    pub points: Vec<(u64, f64)>,
    /// Change points not kept because the channel reached its cap.
    pub dropped: u64,
}

/// `result.json`'s `analog_series` block: every analog channel over the run.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct AnalogSeries {
    /// Simulated time of the last sample any model wrote.
    pub end_time_ns: u64,
    /// One entry per channel, in channel-table order.
    pub channels: Vec<AnalogSeriesChannel>,
}

/// Exact running record of one channel inside one time window.
///
/// The ring keeps the newest rows and drops the oldest. A tally sees every
/// value when it is written, so its answers stay exact for a run of any
/// length. The channel is sample-and-hold: the value it holds at `from_ns`
/// is part of the window.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AnalogWindowTally {
    pub channel: usize,
    /// The window is `from_ns <= time_ns <= to_ns`.
    pub from_ns: u64,
    pub to_ns: u64,
    /// `[time_ns, value]` of the lowest value in the window, first occurrence.
    /// A value held from before the window has the time `from_ns`.
    pub min: Option<(u64, f32)>,
    /// `[time_ns, value]` of the highest value in the window.
    pub max: Option<(u64, f32)>,
    /// The newest sample written at or before `to_ns`, with its own time.
    pub last: Option<(u64, f32)>,
    /// Level crossings inside the window, when a level was requested.
    pub crossings: u64,
    /// The first crossing times, at most the requested number. A crossing has
    /// the time of the first sample at or past the level; nothing is
    /// interpolated between samples.
    pub crossing_times_ns: Vec<u64>,
    #[serde(skip)]
    level: Option<(f32, AnalogCrossing)>,
    #[serde(skip)]
    max_times: usize,
    /// The value before `newest`.
    #[serde(skip)]
    previous: Option<f32>,
    /// The newest value. Writes at 0 ns replace each other, as in the ring.
    #[serde(skip)]
    newest: Option<(u64, f32)>,
    #[serde(skip)]
    entered: bool,
}

/// The direction of a level crossing an [`AnalogWindowTally`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalogCrossing {
    /// From below the level to at or above it.
    Rising,
    /// From above the level to at or below it.
    Falling,
}

impl AnalogWindowTally {
    fn note(&mut self, time_ns: u64, value: f32) {
        if self.min.is_none_or(|(_, min)| value < min) {
            self.min = Some((time_ns, value));
        }
        if self.max.is_none_or(|(_, max)| value > max) {
            self.max = Some((time_ns, value));
        }
    }

    /// The window starts with the value the channel already holds.
    fn enter(&mut self, time_ns: u64) {
        if !self.entered {
            self.entered = true;
            if let Some(held) = self.previous.filter(|_| time_ns > self.from_ns) {
                self.note(self.from_ns, held);
            }
        }
    }

    /// Account for `newest`; no later write can replace it.
    fn commit(&mut self) {
        let Some((time_ns, value)) = self.newest.take() else {
            return;
        };
        if time_ns <= self.to_ns {
            self.last = Some((time_ns, value));
        }
        if time_ns > self.to_ns {
            self.enter(time_ns);
        } else if time_ns >= self.from_ns {
            self.enter(time_ns);
            self.note(time_ns, value);
            let crossed = match (self.previous, self.level) {
                (Some(before), Some((level, AnalogCrossing::Rising))) => {
                    before < level && value >= level
                }
                (Some(before), Some((level, AnalogCrossing::Falling))) => {
                    before > level && value <= level
                }
                _ => false,
            };
            if crossed {
                self.crossings += 1;
                if self.crossing_times_ns.len() < self.max_times {
                    self.crossing_times_ns.push(time_ns);
                }
            }
        }
        self.previous = Some(value);
    }

    fn write(&mut self, time_ns: u64, value: f32) {
        if !matches!(self.newest, Some((0, _)) if time_ns == 0) {
            self.commit();
        }
        self.newest = Some((time_ns, value));
    }

    /// The tally as it stands when the run has reached `end_ns`.
    fn at(&self, end_ns: u64) -> Self {
        let mut tally = self.clone();
        tally.commit();
        if end_ns >= tally.from_ns {
            tally.enter(u64::MAX);
        }
        tally
    }
}

/// One column of the trace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnalogChannel {
    /// Channel name — the probe name, or the `trace:` expression, prefixed
    /// with `<model id>.` when the adapter is registered on a runner.
    pub name: String,
    /// Engineering unit such as `"V"`, `"mV"`, `"A"`, or `"rpm"`.
    pub unit: String,
}

impl AnalogChannel {
    /// A volts channel.
    pub fn volts(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            unit: "V".to_string(),
        }
    }

    /// An amps channel.
    pub fn amps(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            unit: "A".to_string(),
        }
    }
}

/// One row: every channel's value at one instant.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnalogSample {
    /// Co-simulation step index of the model that wrote this row. The
    /// operating point is step 0.
    pub cycle: u64,
    /// Simulated time at the END of the step that produced the row, matching
    /// [`crate::cosim::CosimStep::time_ns`].
    pub time_ns: u64,
    /// One value per entry of [`AnalogTraceBatch::channels`], in order.
    pub values: Vec<f32>,
}

/// What a reader gets back from a cursor read.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct AnalogTraceBatch {
    /// The channel table; stable for the life of the run.
    pub channels: Vec<AnalogChannel>,
    /// Samples newer than the requested cursor, oldest first.
    pub samples: Vec<AnalogSample>,
    /// Cursor to pass to the next read.
    pub next_cursor: u64,
    /// Samples dropped by ring overflow since the run started. Non-zero means
    /// the reader fell behind and the waveform has a gap before `samples`.
    pub dropped: u64,
}

impl AnalogTraceBatch {
    /// `time_ns,<channel>...`, one row per sample: the file `labwired test
    /// --analog-trace x.csv` writes, and what the multimeter readings are
    /// computed from. Times are the engine's own nanoseconds.
    pub fn write_csv<W: std::io::Write>(&self, mut sink: W) -> std::io::Result<()> {
        write!(sink, "time_ns")?;
        for channel in &self.channels {
            write!(sink, ",{}", csv_field(&channel.name))?;
        }
        writeln!(sink)?;
        for sample in &self.samples {
            write!(sink, "{}", sample.time_ns)?;
            for value in &sample.values {
                write!(sink, ",{value}")?;
            }
            writeln!(sink)?;
        }
        Ok(())
    }
}

/// A channel name as a CSV field: quoted only when it has to be.
fn csv_field(name: &str) -> String {
    if name.contains([',', '"', '\n']) {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        name.to_string()
    }
}

/// The ring itself.
#[derive(Debug)]
pub struct AnalogTrace {
    channels: Vec<AnalogChannel>,
    samples: VecDeque<(u64, AnalogSample)>,
    capacity: usize,
    next_seq: u64,
    dropped: u64,
    last_values: Vec<f32>,
    series: Vec<AnalogSeriesChannel>,
    series_cap: usize,
    end_time_ns: u64,
    tallies: Vec<AnalogWindowTally>,
}

impl Default for AnalogTrace {
    fn default() -> Self {
        Self::new(DEFAULT_TRACE_SAMPLES)
    }
}

impl AnalogTrace {
    /// A ring holding `capacity` samples (at least one).
    pub fn new(capacity: usize) -> Self {
        Self {
            channels: Vec::new(),
            samples: VecDeque::new(),
            capacity: capacity.max(1),
            next_seq: 0,
            dropped: 0,
            last_values: Vec::new(),
            series: Vec::new(),
            series_cap: DEFAULT_SERIES_POINTS,
            end_time_ns: 0,
            tallies: Vec::new(),
        }
    }

    /// Start an exact tally of `channel` with `from_ns <= time_ns <= to_ns`.
    /// With `level`, it also counts the crossings of that level in one
    /// direction and keeps at most `max_times` crossing times. Returns the
    /// tally id. A tally cannot see the past, so it must start before
    /// simulated time advances.
    pub fn tally_window(
        &mut self,
        channel: usize,
        from_ns: u64,
        to_ns: u64,
        level: Option<(f32, AnalogCrossing)>,
        max_times: usize,
    ) -> Result<usize, String> {
        if self.end_time_ns != 0 || self.dropped != 0 {
            return Err("an analog tally must start before simulated time advances".to_owned());
        }
        if channel >= self.channels.len() {
            return Err(format!("unknown analog channel index {channel}"));
        }
        if from_ns > to_ns {
            return Err("analog tally window is empty".to_owned());
        }
        if level.is_some_and(|(level, _)| !level.is_finite()) {
            return Err("analog tally level must be finite".to_owned());
        }
        self.tallies.push(AnalogWindowTally {
            channel,
            from_ns,
            to_ns,
            min: None,
            max: None,
            last: None,
            crossings: 0,
            crossing_times_ns: Vec::new(),
            level,
            max_times,
            previous: None,
            // The resolved initial row, when the models already wrote it.
            newest: (!self.samples.is_empty()).then(|| (0, self.last_values[channel])),
            entered: false,
        });
        Ok(self.tallies.len() - 1)
    }

    /// Tally `id` as it stands when the run has reached `end_ns`.
    pub fn window_tally(&self, id: usize, end_ns: u64) -> Option<AnalogWindowTally> {
        self.tallies.get(id).map(|tally| tally.at(end_ns))
    }

    /// Cap the run-long series at `points` per channel (at least two).
    pub fn set_series_cap(&mut self, points: usize) {
        self.series_cap = points.max(2);
    }

    /// The run-long change-point series of every channel.
    pub fn series(&self) -> AnalogSeries {
        AnalogSeries {
            end_time_ns: self.end_time_ns,
            channels: self.series.clone(),
        }
    }

    /// Ring depth in samples.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Raise the ring depth to at least `capacity`. Called when a model
    /// declaring a deeper `trace_samples` joins a ring that already exists.
    pub fn reserve_capacity(&mut self, capacity: usize) {
        self.capacity = self.capacity.max(capacity.max(1));
    }

    /// Claim a contiguous block of channels, returning the index of the first.
    pub fn register_channels(&mut self, channels: &[AnalogChannel]) -> usize {
        let base = self.channels.len();
        self.channels.extend_from_slice(channels);
        self.last_values.resize(self.channels.len(), 0.0);
        self.series
            .extend(channels.iter().map(|channel| AnalogSeriesChannel {
                name: channel.name.clone(),
                unit: channel.unit.clone(),
                points: Vec::new(),
                dropped: 0,
            }));
        for sample in &mut self.samples {
            sample.1.values.resize(self.channels.len(), 0.0);
        }
        base
    }

    /// The channel table.
    pub fn channels(&self) -> &[AnalogChannel] {
        &self.channels
    }

    /// Number of samples currently retained.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether the ring holds no samples.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Append one row: `values` fill the block starting at `base`, every other
    /// channel carries its last value forward. Writes at 0 ns merge into one
    /// initial row because model initialization does not advance simulated time.
    pub fn push(&mut self, base: usize, cycle: u64, time_ns: u64, values: &[f64]) {
        for (offset, value) in values.iter().enumerate() {
            if let Some(slot) = self.last_values.get_mut(base + offset) {
                *slot = *value as f32;
            }
            if let Some(channel) = self.series.get_mut(base + offset) {
                record_change(channel, self.series_cap, time_ns, *value);
            }
        }
        for tally in &mut self.tallies {
            if let Some(value) = tally
                .channel
                .checked_sub(base)
                .and_then(|offset| values.get(offset))
            {
                tally.write(time_ns, *value as f32);
            }
        }
        self.end_time_ns = self.end_time_ns.max(time_ns);
        if time_ns == 0 {
            if let Some((_, sample)) = self
                .samples
                .back_mut()
                .filter(|(_, sample)| sample.time_ns == 0)
            {
                sample.values.clone_from(&self.last_values);
                return;
            }
        }
        let sample = AnalogSample {
            cycle,
            time_ns,
            values: self.last_values.clone(),
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        self.samples.push_back((seq, sample));
        while self.samples.len() > self.capacity {
            self.samples.pop_front();
            self.dropped += 1;
        }
    }

    /// Samples with a sequence number at or after `cursor`.
    pub fn snapshot(&self, cursor: u64) -> AnalogTraceBatch {
        let samples: Vec<AnalogSample> = self
            .samples
            .iter()
            .filter(|(seq, _)| *seq >= cursor)
            .map(|(_, sample)| sample.clone())
            .collect();
        AnalogTraceBatch {
            channels: self.channels.clone(),
            samples,
            next_cursor: self.next_seq,
            dropped: self.dropped,
        }
    }
}

/// Append `value` to `channel` when it left the deadband of the last point.
pub(crate) fn record_change(
    channel: &mut AnalogSeriesChannel,
    cap: usize,
    time_ns: u64,
    value: f64,
) {
    if !value.is_finite() {
        return;
    }
    if let Some(&(_, held)) = channel.points.last() {
        let floor = match channel.unit.as_str() {
            "A" => SERIES_DEADBAND_AMPS,
            "rpm" => SERIES_DEADBAND_RPM,
            _ => SERIES_DEADBAND_VOLTS,
        };
        let band = floor.max(SERIES_DEADBAND_REL * held.abs());
        // Arriving at exactly zero (a shaft dry friction stopped, a switched-off
        // rail) is a state, not noise: always a point, so the series never
        // holds a last sub-deadband crawl for a value that reached rest.
        let came_to_rest = value == 0.0 && held != 0.0;
        if !came_to_rest && (value - held).abs() <= band {
            return;
        }
    }
    if channel.points.len() >= cap {
        channel.dropped += 1;
        return;
    }
    channel.points.push((time_ns, value));
}

/// Shared handle to the ring. The adapter lives inside the co-sim runner, so
/// the runner and the machine hold clones of this rather than a borrow.
pub type AnalogTraceHandle = Arc<Mutex<AnalogTrace>>;

/// What a [`crate::Machine`] is handed so an instrument can read the analog
/// trace without knowing where the co-sim runner lives.
///
/// Cloning it is cheap and shares the ring. A machine with no analog model
/// attached has no registry at all and answers an empty batch — the instrument
/// then draws nothing instead of a fabricated flat line.
#[derive(Debug, Clone)]
pub struct AnalogTraceRegistry {
    trace: AnalogTraceHandle,
}

impl Default for AnalogTraceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalogTraceRegistry {
    /// A registry over a fresh, empty ring.
    pub fn new() -> Self {
        Self {
            trace: Arc::new(Mutex::new(AnalogTrace::default())),
        }
    }

    /// The underlying handle, for adapters that append to it.
    pub fn handle(&self) -> AnalogTraceHandle {
        Arc::clone(&self.trace)
    }

    /// The channel table.
    pub fn channels(&self) -> Vec<AnalogChannel> {
        match self.trace.lock() {
            Ok(trace) => trace.channels().to_vec(),
            Err(poisoned) => poisoned.into_inner().channels().to_vec(),
        }
    }

    /// Samples newer than `cursor`.
    pub fn snapshot(&self, cursor: u64) -> AnalogTraceBatch {
        match self.trace.lock() {
            Ok(trace) => trace.snapshot(cursor),
            Err(poisoned) => poisoned.into_inner().snapshot(cursor),
        }
    }

    /// See [`AnalogTrace::tally_window`].
    pub fn tally_window(
        &self,
        channel: usize,
        from_ns: u64,
        to_ns: u64,
        level: Option<(f32, AnalogCrossing)>,
        max_times: usize,
    ) -> Result<usize, String> {
        match self.trace.lock() {
            Ok(mut trace) => trace.tally_window(channel, from_ns, to_ns, level, max_times),
            Err(poisoned) => poisoned
                .into_inner()
                .tally_window(channel, from_ns, to_ns, level, max_times),
        }
    }

    /// See [`AnalogTrace::window_tally`].
    pub fn window_tally(&self, id: usize, end_ns: u64) -> Option<AnalogWindowTally> {
        match self.trace.lock() {
            Ok(trace) => trace.window_tally(id, end_ns),
            Err(poisoned) => poisoned.into_inner().window_tally(id, end_ns),
        }
    }

    /// The run-long change-point series of every channel.
    pub fn series(&self) -> AnalogSeries {
        match self.trace.lock() {
            Ok(trace) => trace.series(),
            Err(poisoned) => poisoned.into_inner().series(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_values_from_several_models_share_one_resolved_sample() {
        let mut trace = AnalogTrace::new(8);
        let left = trace.register_channels(&[AnalogChannel::volts("left.voltage")]);
        let right = trace.register_channels(&[AnalogChannel::volts("right.voltage")]);

        trace.push(left, 0, 0, &[2.3]);
        trace.push(right, 0, 0, &[0.4]);

        let batch = trace.snapshot(0);
        assert_eq!(batch.channels.len(), 2);
        assert_eq!(batch.samples.len(), 1);
        assert_eq!(batch.samples[0].time_ns, 0);
        assert_eq!(batch.samples[0].values, [2.3, 0.4]);
        assert_eq!(batch.next_cursor, 1);
        assert_eq!(batch.dropped, 0);
    }

    #[test]
    fn a_window_tally_stays_exact_after_the_ring_overflows() {
        let mut trace = AnalogTrace::new(4);
        let other = trace.register_channels(&[AnalogChannel::volts("other.v")]);
        let ramp = trace.register_channels(&[AnalogChannel::volts("ramp.v")]);
        // Initialization writes the 0 ns row twice; the second value stands.
        trace.push(ramp, 0, 0, &[9.0]);
        trace.push(ramp, 0, 0, &[0.0]);
        trace.push(other, 0, 0, &[5.0]);

        let whole = trace.tally_window(ramp, 0, u64::MAX, None, 0).unwrap();
        let rising = trace
            .tally_window(ramp, 0, u64::MAX, Some((2.0, AnalogCrossing::Rising)), 2)
            .unwrap();
        let falling = trace
            .tally_window(ramp, 0, u64::MAX, Some((2.0, AnalogCrossing::Falling)), 8)
            .unwrap();
        // 250..=450 ns starts between two samples and ends between two.
        let window = trace
            .tally_window(ramp, 250, 450, Some((2.0, AnalogCrossing::Rising)), 8)
            .unwrap();
        let at = trace.tally_window(ramp, 250, 250, None, 0).unwrap();
        assert!(trace.tally_window(7, 0, 1, None, 0).is_err());
        assert!(trace.tally_window(ramp, 2, 1, None, 0).is_err());
        assert!(trace
            .tally_window(ramp, 0, 1, Some((f32::NAN, AnalogCrossing::Rising)), 0)
            .is_err());

        // A late 0 ns write replaces the initial value and is not a sample.
        trace.push(ramp, 0, 0, &[7.0]);
        trace.push(ramp, 0, 0, &[0.0]);
        // A triangle 0,1,2,3,2,1,0,1,... V, one sample per 100 ns, while the
        // other model writes rows that carry this channel forward.
        let triangle = |step: u64| match step % 6 {
            phase @ 0..=3 => phase as f64,
            phase => (6 - phase) as f64,
        };
        for step in 1..=600u64 {
            trace.push(ramp, step, step * 100, &[triangle(step)]);
            trace.push(other, step, step * 100 + 50, &[5.0]);
        }
        assert_eq!(trace.len(), 4);
        assert!(trace.snapshot(0).dropped > 1_000);

        let end = 600 * 100 + 50;
        let tally = trace.window_tally(whole, end).unwrap();
        assert_eq!((tally.min, tally.max), (Some((0, 0.0)), Some((300, 3.0))));
        assert_eq!(tally.last, Some((60_000, 0.0)));
        // One period is 600 ns: up through 2 V at 200 ns, down through it at 400 ns.
        let tally = trace.window_tally(rising, end).unwrap();
        assert_eq!(
            (tally.crossings, tally.crossing_times_ns),
            (100, vec![200, 800])
        );
        let tally = trace.window_tally(falling, end).unwrap();
        assert_eq!(tally.crossings, 100);
        assert_eq!(tally.crossing_times_ns[..3], [400, 1000, 1600]);

        // The window holds 2 V from the 200 ns sample, then 3 V and 2 V.
        let tally = trace.window_tally(window, end).unwrap();
        assert_eq!((tally.min, tally.max), (Some((250, 2.0)), Some((300, 3.0))));
        assert_eq!(tally.crossings, 0, "2 V was reached before the window");
        assert_eq!(tally.last, Some((400, 2.0)));
        let tally = trace.window_tally(at, end).unwrap();
        assert_eq!((tally.min, tally.max), (Some((250, 2.0)), Some((250, 2.0))));
        assert_eq!(tally.last, Some((200, 2.0)), "the sample has its own time");

        // A tally cannot see the past.
        assert!(trace.tally_window(ramp, 0, 1, None, 0).is_err());
    }

    #[test]
    fn a_window_the_run_has_not_reached_has_no_observation() {
        let mut trace = AnalogTrace::new(4);
        let channel = trace.register_channels(&[AnalogChannel::volts("v")]);
        trace.push(channel, 0, 0, &[1.5]);
        let later = trace.tally_window(channel, 1_000, 2_000, None, 0).unwrap();
        trace.push(channel, 1, 100, &[2.5]);
        let early = trace.window_tally(later, 999).unwrap();
        assert_eq!((early.min, early.max), (None, None));
        assert_eq!(early.last, Some((100, 2.5)));
        let reached = trace.window_tally(later, 1_000).unwrap();
        assert_eq!(reached.min, Some((1_000, 2.5)));
        assert_eq!(reached.max, Some((1_000, 2.5)));
    }

    #[test]
    fn the_series_outlives_the_ring() {
        let mut trace = AnalogTrace::new(4);
        trace.register_channels(&[AnalogChannel::amps("m.i(L1)")]);
        for step in 0..100u64 {
            let amps = if (20..60).contains(&step) { 0.2 } else { 0.0 };
            trace.push(0, step, step * 100_000, &[amps]);
        }
        assert_eq!(trace.len(), 4, "the scope ring keeps only its depth");
        let series = trace.series();
        assert_eq!(series.end_time_ns, 99 * 100_000);
        let points = &series.channels[0].points;
        assert_eq!(
            points.first(),
            Some(&(0, 0.0)),
            "the first sample is always a point"
        );
        assert!(
            points.contains(&(20 * 100_000, 0.2)),
            "turn-on kept after the ring wrapped"
        );
        assert!(points.contains(&(60 * 100_000, 0.0)), "turn-off kept");
    }

    #[test]
    fn changes_inside_the_deadband_are_not_points() {
        let mut trace = AnalogTrace::new(8);
        trace.register_channels(&[AnalogChannel::amps("i"), AnalogChannel::volts("v")]);
        trace.push(0, 0, 0, &[0.100, 1.000]);
        trace.push(
            0,
            1,
            1,
            &[
                0.100 + SERIES_DEADBAND_REL * 0.05,
                1.0 + SERIES_DEADBAND_VOLTS * 0.5,
            ],
        );
        trace.push(0, 2, 2, &[0.150, 2.0]);
        let series = trace.series();
        assert_eq!(series.channels[0].points, vec![(0, 0.100), (2, 0.150)]);
        assert_eq!(series.channels[1].points, vec![(0, 1.0), (2, 2.0)]);
    }

    #[test]
    fn reaching_exactly_zero_is_a_point_even_inside_the_deadband() {
        let mut trace = AnalogTrace::new(8);
        trace.register_channels(&[AnalogChannel::amps("i")]);
        trace.push(0, 0, 0, &[0.5]);
        trace.push(0, 1, 1, &[SERIES_DEADBAND_AMPS * 0.5]);
        trace.push(0, 2, 2, &[0.0]);
        trace.push(0, 3, 3, &[0.0]);
        let points = &trace.series().channels[0].points;
        assert_eq!(points.last(), Some(&(2, 0.0)), "{points:?}");
        assert!(!points.contains(&(3, 0.0)), "a held zero is no new point");
    }
}
