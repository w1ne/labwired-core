// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bounded live observations of native numeric signals.
//!
//! This stream is independent of both the co-simulation analog cursor and
//! the historical `analog_series` prefix. It records real observations,
//! including unchanged values, and evicts the oldest point when full. Reads
//! are non-destructive, so each instrument can keep its own next-unread cursor.
//! A new recorder or an explicit timeline reset gets a new epoch. A physical
//! CPU/reset-pin event on a continuing simulation timeline does not.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

/// The existing live analog bound, applied to individual numeric observations.
pub const DEFAULT_LIVE_NUMERIC_POINTS: usize = crate::analog::DEFAULT_TRACE_SAMPLES;

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

fn new_epoch() -> String {
    format!("numeric-{}", NEXT_EPOCH.fetch_add(1, Ordering::Relaxed))
}

/// One signal's descriptor and the interval supported by native observations.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveNumericChannel {
    pub name: String,
    pub unit: String,
    /// False before the first observation or after the producer invalidates it.
    pub valid: bool,
    /// First retained observation in this channel's current validity interval.
    /// None means no retained native point establishes a value for this interval.
    pub known_from_ns: Option<u64>,
    /// Last successful observation of this channel; frozen while invalid.
    pub known_until_ns: Option<u64>,
    pub invalid_reason: Option<String>,
}

/// One actual native observation. Channel is a zero-based descriptor index.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveNumericPoint {
    pub seq: u64,
    pub channel: usize,
    pub time_ns: u64,
    pub value: f64,
}

/// A cursor read of the live stream. Sequence numbers count individual points.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveNumericBatch {
    pub epoch: String,
    pub oldest_cursor: u64,
    /// Producer's exclusive end, even when max_points trims the returned data.
    pub next_cursor: u64,
    /// Requested points already overwritten by the native ring.
    pub reader_missed: u64,
    /// All native ring evictions, including points a reader already consumed.
    pub evicted: u64,
    /// Available requested points omitted only by this read's return limit.
    pub omitted: u64,
    /// A cursor beyond this epoch's end cannot establish continuity. Metadata
    /// still identifies the epoch, allowing a replacement-machine reread at 0.
    pub reset_required: bool,
    pub channels: Vec<LiveNumericChannel>,
    pub points: Vec<LiveNumericPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LiveNumericError {
    #[error("live numeric channel name must not be empty")]
    EmptyName,
    #[error("live numeric channel unit changed")]
    ChangedUnit,
    #[error("unknown live numeric channel")]
    UnknownChannel,
    #[error("live numeric observation is not finite")]
    NonFinite,
    #[error("live numeric channel time moved backward")]
    TimeRewind,
}

#[derive(Debug)]
struct ChannelState {
    name: String,
    unit: String,
    valid: bool,
    valid_since_seq: u64,
    known_until_ns: Option<u64>,
    invalid_reason: Option<String>,
}

/// Generic native recorder. There is no motor-specific name or unit policy.
#[derive(Debug)]
pub struct LiveNumericCapture {
    epoch: String,
    channels: Vec<ChannelState>,
    points: VecDeque<LiveNumericPoint>,
    capacity: usize,
    next_seq: u64,
    evicted: u64,
    last_boundary: Option<u64>,
}

impl Default for LiveNumericCapture {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_LIVE_NUMERIC_POINTS)
    }
}

impl LiveNumericCapture {
    fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.clamp(1, DEFAULT_LIVE_NUMERIC_POINTS);
        Self {
            epoch: new_epoch(),
            channels: Vec::new(),
            points: VecDeque::with_capacity(capacity),
            capacity,
            next_seq: 0,
            evicted: 0,
            last_boundary: None,
        }
    }

    /// Register a stable path and unit, or retrieve its existing index. A
    /// caller cannot change a path's unit and reinterpret earlier observations.
    pub fn register_channel(
        &mut self,
        name: impl Into<String>,
        unit: impl Into<String>,
    ) -> Result<usize, LiveNumericError> {
        let name = name.into();
        let unit = unit.into();
        if name.is_empty() {
            return Err(LiveNumericError::EmptyName);
        }
        if let Some(index) = self.channels.iter().position(|c| c.name == name) {
            return if self.channels[index].unit == unit {
                Ok(index)
            } else {
                Err(LiveNumericError::ChangedUnit)
            };
        }
        let index = self.channels.len();
        self.channels.push(ChannelState {
            name,
            unit,
            valid: false,
            valid_since_seq: self.next_seq,
            known_until_ns: None,
            invalid_reason: None,
        });
        Ok(index)
    }

    /// Append a real observation, even when it equals the preceding value.
    /// Invalid values/timestamps never enter the ring or advance the horizon.
    pub fn observe(
        &mut self,
        channel: usize,
        time_ns: u64,
        value: f64,
    ) -> Result<(), LiveNumericError> {
        let state = self
            .channels
            .get_mut(channel)
            .ok_or(LiveNumericError::UnknownChannel)?;
        let error = if !value.is_finite() {
            Some(LiveNumericError::NonFinite)
        } else if state.known_until_ns.is_some_and(|last| time_ns < last) {
            Some(LiveNumericError::TimeRewind)
        } else {
            None
        };
        if let Some(error) = error {
            state.valid = false;
            state.invalid_reason = Some(error.to_string());
            return Err(error);
        }
        if !state.valid {
            state.valid_since_seq = self.next_seq;
        }
        state.valid = true;
        state.invalid_reason = None;
        state.known_until_ns = Some(time_ns);
        let point = LiveNumericPoint {
            seq: self.next_seq,
            channel,
            time_ns,
            value,
        };
        self.next_seq += 1;
        if self.points.len() == self.capacity {
            self.points.pop_front();
            self.evicted += 1;
        }
        self.points.push_back(point);
        Ok(())
    }

    /// Stop treating this channel as a live measurement. Retain its last
    /// successful observation time without lending it another channel's time.
    pub fn invalidate(&mut self, channel: usize, reason: &str) {
        if let Some(state) = self.channels.get_mut(channel) {
            state.valid = false;
            if state.invalid_reason.as_deref() != Some(reason) {
                state.invalid_reason = Some(reason.to_owned());
            }
        }
    }

    /// Clear observations for an actual timeline replacement/rewind, retaining
    /// descriptor indices. A continuous-time physical CPU reset does not call it.
    pub fn reset_timeline(&mut self) {
        self.epoch = new_epoch();
        self.points.clear();
        self.next_seq = 0;
        self.evicted = 0;
        self.last_boundary = None;
        for state in &mut self.channels {
            state.valid = false;
            state.valid_since_seq = 0;
            state.known_until_ns = None;
            state.invalid_reason = None;
        }
    }

    /// A producer's monotonic native boundary (for example a machine cycle).
    /// Detect an actual rewind without changing the producer's physical state
    /// or scheduling anchors. Channels may use different native timebases.
    pub fn begin_boundary(&mut self, boundary: u64) {
        if self
            .last_boundary
            .is_some_and(|previous| boundary < previous)
        {
            self.reset_timeline();
        }
        self.last_boundary = Some(boundary);
    }

    /// Return seq >= cursor, limited to the newest max_points observations.
    /// Neither a read nor its return limit changes producer state or capacity.
    pub fn snapshot(&self, cursor: u64, max_points: usize) -> LiveNumericBatch {
        let oldest_cursor = self.points.front().map_or(self.next_seq, |p| p.seq);
        let reset_required = cursor > self.next_seq;
        let available = self.next_seq.saturating_sub(cursor.max(oldest_cursor)) as usize;
        let count = available.min(max_points);
        let points = self
            .points
            .iter()
            .skip(self.points.len() - count)
            .copied()
            .collect();
        let mut known_from = vec![None; self.channels.len()];
        for point in &self.points {
            let state = &self.channels[point.channel];
            if point.seq >= state.valid_since_seq && known_from[point.channel].is_none() {
                known_from[point.channel] = Some(point.time_ns);
            }
        }
        let channels = self
            .channels
            .iter()
            .enumerate()
            .map(|(index, state)| LiveNumericChannel {
                name: state.name.clone(),
                unit: state.unit.clone(),
                valid: state.valid,
                known_from_ns: known_from[index],
                known_until_ns: state.known_until_ns,
                invalid_reason: state.invalid_reason.clone(),
            })
            .collect();
        LiveNumericBatch {
            epoch: self.epoch.clone(),
            oldest_cursor,
            next_cursor: self.next_seq,
            reader_missed: oldest_cursor.saturating_sub(cursor),
            evicted: self.evicted,
            omitted: (available - count) as u64,
            reset_required,
            channels,
            points,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_points_survive_wrap_and_independent_readers_report_exact_loss() {
        let mut capture = LiveNumericCapture::with_capacity(3);
        let ch = capture.register_channel("plant.current", "A").unwrap();
        for time in 0..3 {
            capture.observe(ch, time, time as f64).unwrap();
        }
        let fast = capture.snapshot(0, 3);
        for time in 3..5 {
            capture.observe(ch, time, time as f64).unwrap();
        }
        let slow = capture.snapshot(0, 2);
        assert_eq!((slow.oldest_cursor, slow.next_cursor), (2, 5));
        assert_eq!((slow.evicted, slow.reader_missed, slow.omitted), (2, 2, 1));
        assert_eq!(
            slow.points.iter().map(|p| p.seq).collect::<Vec<_>>(),
            [3, 4]
        );
        assert_eq!(
            slow,
            capture.snapshot(0, 2),
            "reads never consume another reader's data"
        );
        let next = capture.snapshot(fast.next_cursor, 3);
        assert_eq!((next.reader_missed, next.omitted, next.evicted), (0, 0, 2));
        assert_eq!(next.points, slow.points);
        assert!(capture.snapshot(next.next_cursor, 3).points.is_empty());
        assert_eq!(slow.channels[0].known_from_ns, Some(2));
        assert_eq!(slow.channels[0].known_until_ns, Some(4));
    }

    #[test]
    fn quiet_observations_and_native_f64_values_survive_late_subscription() {
        let mut capture = LiveNumericCapture::with_capacity(2);
        let ch = capture.register_channel("sensor.signal", "V").unwrap();
        let value = 1.0 + f64::EPSILON;
        for time in 0..5 {
            capture.observe(ch, time, value).unwrap();
        }
        let batch = capture.snapshot(0, 1);
        assert_eq!(
            batch.points,
            [LiveNumericPoint {
                seq: 4,
                channel: ch,
                time_ns: 4,
                value
            }]
        );
        assert_eq!(batch.channels[0].known_until_ns, Some(4));
        assert_eq!(batch.channels[0].known_from_ns, Some(3));
        assert_eq!((batch.reader_missed, batch.omitted), (3, 1));
    }

    #[test]
    fn failed_channel_keeps_its_horizon_and_resume_starts_a_new_interval() {
        let mut capture = LiveNumericCapture::with_capacity(4);
        let failed = capture.register_channel("failed", "A").unwrap();
        let healthy = capture.register_channel("healthy", "rpm").unwrap();
        let unknown = capture.register_channel("not-observed", "V").unwrap();
        capture.observe(failed, 10, 2.0).unwrap();
        capture.invalidate(failed, "integration failed");
        capture.observe(healthy, 20, 10.0).unwrap();
        let before = capture.snapshot(0, 4);
        assert!(!before.channels[failed].valid);
        assert_eq!(before.channels[failed].known_until_ns, Some(10));
        assert_eq!(before.channels[healthy].known_until_ns, Some(20));
        assert_eq!(before.channels[unknown].known_from_ns, None);
        assert!(!before.channels[unknown].valid);
        capture.observe(failed, 30, 3.0).unwrap();
        let after = capture.snapshot(0, 4);
        assert!(after.channels[failed].valid);
        assert_eq!(after.channels[failed].known_from_ns, Some(30));
        assert_eq!(after.channels[failed].known_until_ns, Some(30));
        assert_eq!(after.channels[failed].invalid_reason, None);
    }

    #[test]
    fn invalid_observations_do_not_advance_sequence_or_horizon() {
        let mut capture = LiveNumericCapture::with_capacity(2);
        let ch = capture.register_channel("signal", "V").unwrap();
        capture.observe(ch, 10, 1.0).unwrap();
        assert_eq!(
            capture.observe(ch, 20, f64::NAN),
            Err(LiveNumericError::NonFinite)
        );
        assert_eq!(
            capture.observe(ch, 5, 1.0),
            Err(LiveNumericError::TimeRewind)
        );
        let batch = capture.snapshot(0, 2);
        assert_eq!(batch.next_cursor, 1);
        assert_eq!(batch.channels[0].known_until_ns, Some(10));
        assert!(!batch.channels[0].valid);
        assert!(batch.points.iter().all(|p| p.value.is_finite()));
        assert_eq!(
            capture.register_channel("signal", "A"),
            Err(LiveNumericError::ChangedUnit)
        );
    }

    #[test]
    fn rewind_and_replacement_have_new_epochs_and_no_old_cursor_continuity() {
        let mut capture = LiveNumericCapture::with_capacity(2);
        let ch = capture.register_channel("signal", "V").unwrap();
        capture.observe(ch, 100, 1.0).unwrap();
        let old = capture.snapshot(0, 2);
        let replacement = LiveNumericCapture::with_capacity(2);
        assert_ne!(old.epoch, replacement.snapshot(0, 2).epoch);
        capture.reset_timeline();
        let reset = capture.snapshot(old.next_cursor, 2);
        assert_ne!(old.epoch, reset.epoch);
        assert!(reset.reset_required);
        assert_eq!(
            (reset.oldest_cursor, reset.next_cursor, reset.evicted),
            (0, 0, 0)
        );
        assert!(reset.points.is_empty());
        assert_eq!(reset.channels[0].known_until_ns, None);
        capture.observe(ch, 1, 2.0).unwrap();
        assert_eq!(capture.snapshot(0, 2).points[0].time_ns, 1);
        assert!(
            capture.snapshot(2, 2).reset_required,
            "same-epoch future cursor is explicit"
        );
    }

    #[test]
    fn only_a_true_boundary_rewind_changes_epoch() {
        let mut capture = LiveNumericCapture::with_capacity(2);
        let ch = capture.register_channel("signal", "V").unwrap();
        capture.begin_boundary(10);
        capture.observe(ch, 100, 1.0).unwrap();
        let old = capture.snapshot(0, 2);
        capture.begin_boundary(10);
        capture.begin_boundary(11);
        assert_eq!(old.epoch, capture.snapshot(0, 2).epoch);
        capture.begin_boundary(2);
        let reset = capture.snapshot(0, 2);
        assert_ne!(old.epoch, reset.epoch);
        assert_eq!(reset.next_cursor, 0);
        assert!(!reset.channels[0].valid);
        capture.observe(ch, 20, 2.0).unwrap();
        assert_eq!(capture.snapshot(0, 2).points[0].seq, 0);
    }

    #[test]
    fn evicted_channel_has_no_retained_native_anchor() {
        let mut capture = LiveNumericCapture::with_capacity(2);
        let old = capture.register_channel("slow", "A").unwrap();
        let fresh = capture.register_channel("fast", "V").unwrap();
        capture.observe(old, 1, 1.0).unwrap();
        capture.observe(fresh, 2, 2.0).unwrap();
        capture.observe(fresh, 3, 3.0).unwrap();
        let batch = capture.snapshot(0, 2);
        assert_eq!(batch.channels[old].known_from_ns, None);
        assert_eq!(batch.channels[old].known_until_ns, Some(1));
        assert!(batch.points.iter().all(|p| p.channel == fresh));
    }

    #[test]
    fn default_bound_keeps_twenty_thousand_points_without_stopping_live_updates() {
        let mut capture = LiveNumericCapture::default();
        let ch = capture.register_channel("signal", "V").unwrap();
        assert_eq!(DEFAULT_LIVE_NUMERIC_POINTS, 20_000);
        for time in 0..20_003 {
            capture.observe(ch, time, time as f64).unwrap();
        }
        let full = capture.snapshot(0, usize::MAX);
        assert_eq!(full.points.len(), 20_000);
        assert_eq!(full.points.first().unwrap().seq, 3);
        assert_eq!(full.points.last().unwrap().value, 20_002.0);
        assert_eq!((full.evicted, full.reader_missed, full.omitted), (3, 3, 0));
        let encoded = serde_json::to_value(&full).unwrap();
        assert_eq!(encoded["nextCursor"], 20_003);
        assert_eq!(encoded["channels"][0]["knownUntilNs"], 20_002);
        assert_eq!(encoded["points"][0]["channel"], ch);
        assert!(encoded.get("dropped").is_none());
    }
}
