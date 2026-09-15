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

/// One column of the trace.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnalogChannel {
    /// Channel name — the probe name, or the `trace:` expression, prefixed
    /// with `<model id>.` when the adapter is registered on a runner.
    pub name: String,
    /// `"V"` or `"A"`.
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

/// The ring itself.
#[derive(Debug)]
pub struct AnalogTrace {
    channels: Vec<AnalogChannel>,
    samples: VecDeque<(u64, AnalogSample)>,
    capacity: usize,
    next_seq: u64,
    dropped: u64,
    last_values: Vec<f32>,
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
}
