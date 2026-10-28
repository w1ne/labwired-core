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
    /// `"V"` or `"A"`; `"rpm"` for a motor speed channel.
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

/// One column of the trace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnalogChannel {
    /// Channel name — the probe name, or the `trace:` expression, prefixed
    /// with `<model id>.` when the adapter is registered on a runner.
    pub name: String,
    /// `"V"` or `"A"`; `"rpm"` for a motor speed channel.
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
        }
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
    /// channel carries its last value forward.
    pub fn push(&mut self, base: usize, cycle: u64, time_ns: u64, values: &[f64]) {
        for (offset, value) in values.iter().enumerate() {
            if let Some(slot) = self.last_values.get_mut(base + offset) {
                *slot = *value as f32;
            }
            if let Some(channel) = self.series.get_mut(base + offset) {
                record_change(channel, self.series_cap, time_ns, *value);
            }
        }
        self.end_time_ns = self.end_time_ns.max(time_ns);
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
