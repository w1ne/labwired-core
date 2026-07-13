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
//! This module moves capture INSIDE the simulation loop, with two per-channel
//! capture modes that produce byte-identical edge streams:
//!
//! * **Push (event-driven)** — the default whenever the watched pad's owning
//!   peripheral is instrumented for it (declared by accepting
//!   [`Peripheral::install_logic_tap`](crate::Peripheral::install_logic_tap)).
//!   Pad state only changes when code writes it, so the write sites themselves
//!   report the new level into a shared [`LogicTap`]: GPIO out-latch /
//!   direction / matrix updates, externally driven input-pad updates
//!   (`set_gpio_input` / sim-input), and bit-engine line drivers (the ESP32-C3
//!   I²C `I2cLineLevels` cell). The run loop keeps its full instruction batch
//!   width and idle fast-forward stays available — probing costs (almost)
//!   nothing.
//! * **Poll (per-cycle sampling)** — the fallback for watched pads whose
//!   peripheral is NOT push-instrumented. [`Machine`](crate::Machine) samples
//!   those pads at EVERY engine-cycle boundary; while at least one polled
//!   channel is armed the run loop clamps its batch to one instruction and
//!   idle fast-forward is disabled (the pre-push behaviour, kept as the
//!   honest fallback).
//!
//! Both modes record a `{ch, cycle, value}` edge ONLY on a transition, and
//! timestamps are engine cycles, so identical firmware + identical watch
//! config produce byte-identical edge streams regardless of how the host
//! drives stepping — and regardless of the capture mode. The differential
//! oracle test (`tests/logic_capture_differential.rs`) runs the same firmware
//! under forced poll and under push and asserts the streams are byte-equal.
//!
//! ## Observation semantics (shared by both modes)
//!
//! An edge is *observed* at the first engine-cycle boundary at or after the
//! write that caused it: a write during the instruction executing between
//! cycles `c` and `c+1` yields an edge stamped `c+1` (plus any peripheral
//! tick-cost cycles charged at that boundary — exactly where the poll loop
//! samples). A pad that toggles more than once within one cycle records only
//! the net transition (the last written level wins), matching what a
//! boundary sampler can see. When several channels transition in the same
//! cycle, edges are emitted in ascending channel order; if both push and poll
//! channels transition in the same cycle, the push channels' edges are
//! recorded first (again in ascending channel order). All of this is
//! deterministic — no host timing leaks in.
//!
//! The capture guarantee: any pad level that persists across at least one
//! instruction boundary is captured — no aliasing, at any toggle rate. A pad
//! toggling every cycle yields an edge every cycle (and fills the ring in
//! [`LOGIC_RING_CAPACITY`] cycles if never acknowledged; overflow drops the
//! OLDEST edges and counts them, it never distorts the newest). Earlier versions
//! sampled on a fixed 16-cycle grid, which aliased anything toggling faster
//! than ~32 cycles — bit-banged buses looked wrong before they looked dropped.
//!
//! Everything here is host-frame-independent and allocation-free on the hot
//! path — the single per-step cost when nothing is watched is one `is_empty`
//! check on the channel list (and the edge ring is only ever allocated once a
//! transition is actually recorded).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Maximum number of edges retained in the ring buffer. On overflow the oldest
/// edge is dropped (and counted) so capture never grows without bound. 64k
/// edges is ~10 s of a steadily toggling 100 kHz signal on a single channel
/// before the UI must acknowledge a read — far more than any interactive poll
/// interval.
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
    /// `true` when the owning peripheral pushes this channel's transitions
    /// through the [`LogicTap`] (event-driven capture); `false` for the
    /// per-cycle poll fallback. A channel is exactly one of the two.
    push: bool,
}

/// A single pad-level report pushed by an instrumented peripheral through the
/// [`LogicTap`]. `cycle` is the provisional stamp (the tap clock at write
/// time); [`LogicCapture::ingest_push`] finalises boundary stamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadEvent {
    /// Watch-channel index (see [`LogicEdge::ch`]).
    pub ch: u32,
    /// The pad level after the write.
    pub value: bool,
    /// Provisional engine-cycle stamp: the cycle boundary reached after the
    /// instruction (or peripheral tick) performing the write.
    pub cycle: u64,
}

#[derive(Default)]
struct TapShared {
    /// `true` while at least one push-mode channel is armed. Checked once per
    /// CPU batch (to enable per-instruction clock bumps) and by instrumented
    /// peripherals implicitly via their installed tap state.
    armed: AtomicBool,
    /// The provisional stamp for pushes happening "now": the engine-cycle
    /// boundary reached once the currently-executing instruction retires.
    /// Seeded by [`Machine`](crate::Machine) at batch/step start and advanced
    /// once per retired instruction by the CPU while armed (see
    /// `Cpu::step_batch`), so an MMIO pad write during instruction `k` of a
    /// batch starting at cycle `C` is stamped `C + k + 1` — the same cycle the
    /// per-cycle poll loop would observe it at.
    clock: AtomicU64,
    /// Number of events currently queued — lets the per-boundary drain skip
    /// the mutex entirely on quiet boundaries (the common case), keeping the
    /// armed hot path at one relaxed load per batch.
    pending: AtomicUsize,
    /// Pushed pad events, drained by the machine at observation boundaries.
    /// Single-threaded in practice (everything runs on the machine thread);
    /// the mutex exists because `Peripheral: Send` forces shared handles to be
    /// `Send + Sync`, mirroring `bus_trace::BusTrace`.
    queue: Mutex<Vec<PadEvent>>,
}

/// Shared push-capture tap: the handle instrumented peripherals report pad
/// writes into, and whose clock the CPU advances per retired instruction while
/// push capture is armed. Owned by the bus (one per machine); cloning shares
/// the same underlying state, mirroring [`crate::bus::bus_trace::BusTrace`].
#[derive(Clone, Default)]
pub struct LogicTap {
    shared: Arc<TapShared>,
}

impl std::fmt::Debug for LogicTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogicTap")
            .field("armed", &self.push_armed())
            .finish()
    }
}

impl LogicTap {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` while at least one push-mode channel is armed.
    #[inline]
    pub fn push_armed(&self) -> bool {
        self.shared.armed.load(Ordering::Relaxed)
    }

    /// Advance the provisional stamp by one retired instruction. Called by
    /// CPU batch loops (only while [`push_armed`](Self::push_armed)) BEFORE
    /// executing each instruction, so writes performed by that instruction
    /// stamp with the boundary they become observable at.
    #[inline]
    pub fn bump_clock(&self) {
        self.shared.clock.fetch_add(1, Ordering::Relaxed);
    }

    /// Seed the provisional stamp (machine-side, at batch/step start, after an
    /// idle fast-forward skip, and to "next boundary" after each drain so
    /// pause-time input pushes stamp where the first post-resume sample would
    /// see them).
    #[inline]
    pub fn set_clock(&self, cycle: u64) {
        self.shared.clock.store(cycle, Ordering::Relaxed);
    }

    /// Record a pad level for watched channel `ch`, stamped with the current
    /// provisional clock. Called by instrumented peripherals from their pad
    /// write sites; transitions-only dedup happens at ingest, so reporting an
    /// unchanged level is harmless (but callers avoid it to keep the queue
    /// small).
    pub fn push(&self, ch: u32, value: bool) {
        let cycle = self.shared.clock.load(Ordering::Relaxed);
        self.shared
            .queue
            .lock()
            .unwrap()
            .push(PadEvent { ch, value, cycle });
        self.shared.pending.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn set_armed(&self, armed: bool) {
        self.shared.armed.store(armed, Ordering::Relaxed);
    }

    /// Drain all pushed events (machine-side, at observation boundaries).
    /// A quiet boundary — the common case — costs one relaxed load, no lock,
    /// no allocation.
    pub(crate) fn take_events(&self) -> Vec<PadEvent> {
        if self.shared.pending.load(Ordering::Relaxed) == 0 {
            return Vec::new();
        }
        self.shared.pending.store(0, Ordering::Relaxed);
        std::mem::take(&mut *self.shared.queue.lock().unwrap())
    }

    pub(crate) fn clear_events(&self) {
        self.shared.pending.store(0, Ordering::Relaxed);
        self.shared.queue.lock().unwrap().clear();
    }
}

/// Result of reading the edge ring from a caller cursor.
pub struct LogicEdgeBatch {
    /// Monotonic edge sequence number to pass back on the next read. Doing so
    /// acknowledges all retained edges before it and receives only newer ones.
    pub cursor: u64,
    /// Cumulative count of edges dropped to ring-buffer overflow since the
    /// watch set was installed. Acknowledging an edge never increments this.
    pub dropped: u64,
    /// New edges since the caller's cursor, oldest first.
    pub edges: Vec<LogicEdge>,
}

// ---------------------------------------------------------------------------
// Serialized edge shape (shared with the TS `ChannelEdgeSeries`)
// ---------------------------------------------------------------------------
//
// The hosted/CLI run path serializes captured edges into `result.json` as a
// per-channel step series on the ENGINE-CYCLE axis — byte-shape-identical to
// `packages/board-config`'s `ChannelEdgeSeries` (the ONE definition the browser
// logic-analyzer export and the oracle also consume). `value` and `initial` are
// emitted as 0/1 (never `true`/`false`) to match `EdgeLevel = 0 | 1`. The edges
// themselves are drained from the SAME `LogicCapture::read_edges` the wasm
// `read_logic_edges` accessor uses, so the two surfaces are edge-for-edge
// identical (asserted by the native/wasm parity differential test).

/// One recorded transition on the engine-cycle axis. `value` is 0/1 to match the
/// TS `EdgeLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EdgeTransition {
    pub cycle: u64,
    pub value: u8,
}

/// One watched channel's captured transitions — the Rust mirror of the TS
/// `ChannelEdgeSeries` (`packages/board-config`). `initial` is `Some(0|1)` or
/// `null`; `gaps` lists engine cycles at which the capture ring overflowed
/// (honest "edges lost here" markers — never interpolated over).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChannelEdgeSeries {
    /// Edge-record channel index (the pad's position in the armed watch set).
    pub ch: u32,
    /// Logic-analyzer channel id ("CH0"…).
    pub channel: String,
    pub peripheral: String,
    pub pin: u8,
    /// Pad level at arm time (0/1), or `null` when the engine can't resolve it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<u8>,
    /// Transitions, oldest-first, each stamped with the engine cycle it occurred at.
    pub transitions: Vec<EdgeTransition>,
    /// Engine cycles at which the ring overflowed before this lane's next edge.
    #[serde(default)]
    pub gaps: Vec<u64>,
}

/// The whole logic-edge evidence block embedded in `result.json`. `dropped` is
/// the cumulative ring-overflow count for the run — the oracle FAILS LOUD when
/// it is non-zero (edges were lost, so any period/duty/edge assertion would be
/// evaluated on an incomplete stream).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogicEdgesResult {
    /// Cumulative edges lost to ring-buffer overflow across the whole run.
    pub dropped: u64,
    /// Engine cycle at the end of the run (extends flat traces to "now").
    pub now_cycle: u64,
    pub channels: Vec<ChannelEdgeSeries>,
}

/// Per-channel identity resolved at watch time, paired with the drained edge
/// batch to build a [`LogicEdgesResult`]. `initial[i]`/`peripheral[i]`/`pin[i]`
/// describe channel `i` (the watch-set position, i.e. the edge `ch`).
#[derive(Debug, Clone)]
pub struct LogicChannelMeta {
    pub ch: u32,
    pub peripheral: String,
    pub pin: u8,
    pub initial: Option<bool>,
}

/// Shape a drained [`LogicEdgeBatch`] into the shared per-channel series form.
/// PURE: the edges are grouped by their `ch` onto the pre-resolved channel
/// metadata. Every edge in `batch` lands on exactly one channel (edges whose
/// `ch` has no metadata are impossible — `ch` is a watch-set index — but are
/// dropped defensively rather than panicking). Preserves edge order and value,
/// so flattening the result back to `(ch, cycle, value)` reproduces `batch.edges`
/// exactly (the native/wasm parity property).
pub fn build_logic_edges_result(
    meta: &[LogicChannelMeta],
    batch: &LogicEdgeBatch,
    now_cycle: u64,
) -> LogicEdgesResult {
    let mut channels: Vec<ChannelEdgeSeries> = meta
        .iter()
        .map(|m| ChannelEdgeSeries {
            ch: m.ch,
            channel: format!("CH{}", m.ch),
            peripheral: m.peripheral.clone(),
            pin: m.pin,
            initial: m.initial.map(u8::from),
            transitions: Vec::new(),
            gaps: Vec::new(),
        })
        .collect();
    for edge in &batch.edges {
        if let Some(lane) = channels.iter_mut().find(|c| c.ch == edge.ch) {
            lane.transitions.push(EdgeTransition {
                cycle: edge.cycle,
                value: u8::from(edge.value),
            });
        }
    }
    LogicEdgesResult {
        dropped: batch.dropped,
        now_cycle,
        channels,
    }
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

    /// `true` while at least one resolvable channel is on the per-cycle poll
    /// fallback. Only then does the run loop clamp its batch to one
    /// instruction and disable idle fast-forward; an all-push watch set pays
    /// neither cost.
    #[inline]
    pub fn poll_active(&self) -> bool {
        self.channels
            .iter()
            .any(|c| c.resolved.is_some() && !c.push)
    }

    /// `true` while at least one channel is push-mode (event-driven).
    #[inline]
    pub fn push_active(&self) -> bool {
        self.channels.iter().any(|c| c.push)
    }

    /// Install a fresh watch set from pre-resolved channels, their initial
    /// pad levels and their capture mode, clearing the ring, cursor and drop
    /// count. `resolved[i]`, `initial[i]` and `push[i]` describe channel `i`;
    /// they must be the same length.
    pub fn install(
        &mut self,
        resolved: &[Option<(usize, u8)>],
        initial: &[Option<bool>],
        push: &[bool],
    ) {
        debug_assert_eq!(resolved.len(), initial.len());
        debug_assert_eq!(resolved.len(), push.len());
        self.channels = resolved
            .iter()
            .zip(initial.iter())
            .zip(push.iter())
            .map(|((&resolved, &last), &push)| LogicChannel {
                resolved,
                last,
                push,
            })
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
            if self.channels[i].push {
                continue; // event-driven channel: its peripheral pushes edges
            }
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

    /// Ingest a drained batch of push events, recording transitions with
    /// poll-identical semantics. `boundary` is the engine cycle at the end of
    /// the just-executed instruction batch (before peripheral tick costs) and
    /// `now` is the current engine cycle (after tick costs) — events stamped
    /// AT the boundary are observed at `now`, exactly where the per-cycle poll
    /// loop samples after charging tick costs.
    ///
    /// Semantics, matching a per-cycle boundary sampler bit for bit:
    /// * events are grouped by finalised cycle (the queue is stamped in
    ///   nondecreasing order by construction);
    /// * within one cycle the LAST reported level per channel wins (a pad
    ///   that toggles and returns within one cycle records nothing);
    /// * a cycle's surviving transitions are emitted in ascending channel
    ///   order.
    pub fn ingest_push(&mut self, events: &[PadEvent], boundary: u64, now: u64) {
        let mut i = 0;
        while i < events.len() {
            let cyc = events[i].cycle;
            let mut j = i + 1;
            while j < events.len() && events[j].cycle == cyc {
                j += 1;
            }
            let stamp = if cyc >= boundary { now } else { cyc };
            for ch in 0..self.channels.len() {
                if !self.channels[ch].push {
                    continue;
                }
                // Net level for this channel within the cycle: last write wins.
                let mut level = None;
                for e in &events[i..j] {
                    if e.ch as usize == ch {
                        level = Some(e.value);
                    }
                }
                if let Some(level) = level {
                    if self.channels[ch].last != Some(level) {
                        self.channels[ch].last = Some(level);
                        self.push_edge(LogicEdge {
                            ch: ch as u32,
                            cycle: stamp,
                            value: level,
                        });
                    }
                }
            }
            i = j;
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

    /// Read edges newer than `cursor`. Pass `0` on the first read (right after
    /// `watch`); pass back the returned `cursor` thereafter to acknowledge all
    /// retained edges before it and free their capacity. A stale cursor sees
    /// only the still-retained window; a future cursor is clamped to the newest
    /// recorded edge. Edges lost to capacity overflow are silently skipped —
    /// `dropped` reflects that loss.
    pub fn read_edges(&mut self, cursor: u64) -> LogicEdgeBatch {
        let retained_base = self.next_seq - self.ring.len() as u64;
        let acknowledge_to = cursor.max(retained_base).min(self.next_seq);
        let acknowledged = (acknowledge_to - retained_base) as usize;
        self.ring.drain(..acknowledged);

        let base = self.next_seq - self.ring.len() as u64;
        let start = cursor.max(base).min(self.next_seq);
        let skip = (start - base) as usize;
        let edges = self.ring.iter().skip(skip).copied().collect();
        LogicEdgeBatch {
            cursor: self.next_seq,
            dropped: self.dropped,
            edges,
        }
    }
}
