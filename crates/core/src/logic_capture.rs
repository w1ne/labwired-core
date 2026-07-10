// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Deterministic, in-engine logic-analyzer edge capture.
//!
//! The browser logic analyzer used to sample GPIO pads from the UI thread on
//! `requestAnimationFrame` ticks (via `sample_logic_signals`). Sample spacing
//! therefore depended on host frame timing, so the waveforms aliased and were
//! nondeterministic even though the simulator itself is fully deterministic.
//!
//! This module moves capture INSIDE the simulation loop: while a watch set is
//! active, [`Machine`](crate::Machine) samples the watched pads at EVERY
//! engine-cycle boundary (the run loop executes one instruction per batch
//! while armed) and records a `{ch, cycle, value}` edge ONLY on a transition.
//! Timestamps are engine cycles, so identical firmware + identical watch
//! config produce byte-identical edge streams regardless of how the host
//! drives stepping.
//!
//! The sampling guarantee: any pad level that persists across at least one
//! instruction boundary is captured — no aliasing, at any toggle rate. A pad
//! toggling every cycle yields an edge every cycle (and fills the ring in
//! [`LOGIC_RING_CAPACITY`] cycles if never drained; overflow drops the OLDEST
//! edges and counts them, it never distorts the newest). Earlier versions
//! sampled on a fixed 16-cycle grid, which aliased anything toggling faster
//! than ~32 cycles — bit-banged buses looked wrong before they looked dropped.
//!
//! Everything here is host-frame-independent and allocation-free on the hot
//! path — the single per-step cost when nothing is watched is one `is_empty`
//! check on the channel list (and the edge ring is only ever allocated once a
//! transition is actually recorded).

use std::collections::VecDeque;

/// Maximum number of edges retained in the ring buffer. On overflow the oldest
/// edge is dropped (and counted) so capture never grows without bound. 64k
/// edges is ~10 s of a steadily toggling 100 kHz signal on a single channel
/// before the UI must drain — far more than any interactive poll interval.
pub const LOGIC_RING_CAPACITY: usize = 64 * 1024;

/// A single recorded logic transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LogicEdge {
    /// Channel index — the position of the ref in the watch set passed to
    /// `watch_logic_signals`.
    pub ch: u32,
    /// Engine cycle (`Machine::total_cycles`) at which the transition was
    /// observed.
    pub cycle: u64,
    /// The new pad level after the transition.
    pub value: bool,
}

/// One resolved watch channel. Resolution (peripheral index + pin) happens once
/// at `watch` time so the sampling hot path never does a string lookup.
#[derive(Debug, Clone, Copy)]
struct LogicChannel {
    /// `Some((peripheral_index, pin))` for a resolvable GPIO ref, `None` for an
    /// unresolvable ref (unknown peripheral / non-gpio kind). Unresolvable
    /// channels are never sampled and never emit edges.
    resolved: Option<(usize, u8)>,
    /// Last known pad level, or `None` if never known (initial, or the pad has
    /// only ever read back as unknown). A transition is emitted when a `Some(v)`
    /// read differs from this.
    last: Option<bool>,
}

/// Result of draining the edge ring from a caller cursor.
pub struct LogicEdgeBatch {
    /// Monotonic edge sequence number to pass back on the next read to receive
    /// only newer edges.
    pub cursor: u64,
    /// Cumulative count of edges dropped to ring-buffer overflow since the watch
    /// set was installed.
    pub dropped: u64,
    /// New edges since the caller's cursor, oldest first.
    pub edges: Vec<LogicEdge>,
}

/// In-engine logic-analyzer capture buffer. Owned by [`Machine`](crate::Machine).
#[derive(Default)]
pub struct LogicCapture {
    channels: Vec<LogicChannel>,
    ring: VecDeque<LogicEdge>,
    /// Total edges ever pushed since the watch set was installed. Also the
    /// exclusive upper bound of the cursor space; the oldest retained edge has
    /// sequence `next_seq - ring.len()`.
    next_seq: u64,
    dropped: u64,
}

impl LogicCapture {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` while a watch set is installed. The step loop checks this and
    /// does nothing further when it is `false` — the entire zero-overhead path.
    #[inline]
    pub fn is_active(&self) -> bool {
        !self.channels.is_empty()
    }

    /// Install a fresh watch set from pre-resolved channels and their initial
    /// pad levels, clearing the ring, cursor and drop count. `resolved[i]` and
    /// `initial[i]` describe channel `i`; they must be the same length.
    pub fn install(&mut self, resolved: &[Option<(usize, u8)>], initial: &[Option<bool>]) {
        debug_assert_eq!(resolved.len(), initial.len());
        self.channels = resolved
            .iter()
            .zip(initial.iter())
            .map(|(&resolved, &last)| LogicChannel { resolved, last })
            .collect();
        self.ring.clear();
        self.next_seq = 0;
        self.dropped = 0;
    }

    /// Sample every watched pad at engine cycle `now`, recording a transition
    /// for any channel whose known level changed. `read` maps a resolved
    /// `(peripheral_index, pin)` to the direction-aware pad level (the same
    /// truth as `Peripheral::read_gpio_pad`); a `None` read records nothing.
    ///
    /// Caller guarantees this is only invoked when [`is_active`](Self::is_active).
    /// Every call is a real sample — the step loop invokes this at every cycle
    /// boundary while armed, so no transition that persists across a boundary
    /// is ever missed. Re-sampling the same cycle is harmless: capture is
    /// transitions-only, so an unchanged pad records nothing.
    pub fn sample(&mut self, now: u64, read: impl Fn(usize, u8) -> Option<bool>) {
        for i in 0..self.channels.len() {
            let Some((idx, pin)) = self.channels[i].resolved else {
                continue;
            };
            let Some(level) = read(idx, pin) else {
                continue;
            };
            if self.channels[i].last != Some(level) {
                self.channels[i].last = Some(level);
                self.push_edge(LogicEdge {
                    ch: i as u32,
                    cycle: now,
                    value: level,
                });
            }
        }
    }

    fn push_edge(&mut self, edge: LogicEdge) {
        if self.ring.len() == LOGIC_RING_CAPACITY {
            self.ring.pop_front();
            self.dropped += 1;
        }
        self.ring.push_back(edge);
        self.next_seq += 1;
    }

    /// Drain edges newer than `cursor`. Pass `0` on the first read (right after
    /// `watch`); pass back the returned `cursor` thereafter. Edges older than
    /// the retained window (dropped to overflow) are silently skipped — the
    /// `dropped` count reflects the loss.
    pub fn read_edges(&self, cursor: u64) -> LogicEdgeBatch {
        let base = self.next_seq - self.ring.len() as u64;
        let start = cursor.max(base);
        let skip = (start - base) as usize;
        let edges = self.ring.iter().skip(skip).copied().collect();
        LogicEdgeBatch {
            cursor: self.next_seq,
            dropped: self.dropped,
            edges,
        }
    }
}
