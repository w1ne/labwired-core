// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A timed, fault-confined CAN bus for world nodes.
//!
//! The older medium ([`super::CanBus`]) hands a frame to every other endpoint
//! the instant a controller completes it. That shows two firmwares agree on a
//! protocol and nothing else. Here a frame takes the time the wire takes, and
//! a node on the wrong bit rate behaves as it does on a real bus:
//!
//! * **Timing.** A frame occupies the bus for its real length: SOF,
//!   arbitration, control and data fields, CRC-15, and the stuff bits the
//!   actual bit pattern needs (counted exactly from the frame's bits and its
//!   computed CRC), then CRC delimiter, ACK slot, ACK delimiter, EOF and the
//!   3-bit intermission (ISO 11898-1:2015 §10.4). Time is world picoseconds,
//!   so nothing reaches a receiver earlier than the last EOF bit could.
//! * **Arbitration.** Frames pending when the bus goes idle start together
//!   and the lowest arbitration field wins, bit by bit: dominant (0) beats
//!   recessive, so a standard frame beats an extended one with the same base
//!   identifier (SRR and IDE are recessive), and a data frame beats a remote
//!   frame (§10.5). Losers become receivers and retry when the bus is idle.
//! * **ACK.** A transmitter needs at least one other node on the bus at a
//!   matching bit rate to drive the ACK slot dominant; otherwise it sees an
//!   ACK error (§10.11).
//! * **Bit-rate mismatch.** Each node's bit time is derived from its own
//!   controller registers and clock. A node off the bus rate by more than
//!   [`BITRATE_TOLERANCE`] cannot sample anyone's frame: its own frames end
//!   in error frames and, while it is error-active, its active error flag
//!   destroys everyone else's frames too.
//! * **Fault confinement** (§12.1.4): transmit and receive error counters
//!   with error-active / error-passive / bus-off states and bus-off recovery
//!   after 128 occurrences of 11 consecutive recessive bits.
//!
//! The model is frame-level: there is no bit-level waveform. Where a real
//! error is detected depends on the bit pattern two clocks produce; here an
//! error caused by a bit-rate mismatch is detected [`ERROR_DETECT_BITS`]
//! after SOF (the earliest point a stuff error can occur: SOF plus six equal
//! bits). The error frame then takes the 6-bit error flag, up to 6 more bits
//! of other nodes' superposed flags, the 8-bit delimiter and the intermission.
//!
//! Controller register behaviour (bxCAN `ESR`/`TSR`, FDCAN `ECR`/`PSR`) is in
//! the peripheral models; this module is only the medium. The world polls
//! each controller at round ends ([`CanNodePoll`]) and applies the updates
//! the medium schedules ([`CanNodeUpdate`]) at their exact world time.

use super::CanFrame;
use crate::network::timed_uart::{cycles_to_ps, PS_PER_SEC};
use serde::Serialize;
use std::collections::VecDeque;

/// Bus bit rate when the interconnect does not set `config.bitrate`.
pub const DEFAULT_BITRATE: u64 = 500_000;

/// Largest relative bit-rate difference at which a node still samples the
/// bus correctly. CAN resynchronises on recessive-to-dominant edges and
/// tolerates oscillator error up to about 1.58 % at the most permissive bit
/// timing (ISO 11898-1:2015 §11.3.2.2); 1.5 % stays inside it.
pub const BITRATE_TOLERANCE: f64 = 0.015;

/// Bits from SOF to the detection of an error caused by a bit-rate mismatch:
/// SOF plus six equal bits, the earliest a stuff error can be seen
/// (§10.9.2). An approximation, see the module docs.
pub const ERROR_DETECT_BITS: u64 = 7;
/// Active or passive error flag (§10.4.4.2).
pub const ERROR_FLAG_BITS: u64 = 6;
/// Other nodes' error flags, which start when they see the first one and so
/// overlap it by up to six bits (§10.4.4.2, "superposition").
pub const ERROR_ECHO_BITS: u64 = 6;
/// Error delimiter: eight recessive bits (§10.4.4.3).
pub const ERROR_DELIMITER_BITS: u64 = 8;
/// Interframe space intermission (§10.4.6.2).
pub const INTERMISSION_BITS: u64 = 3;
/// An error-passive node waits eight more recessive bits after its own
/// transmission before it may start another ("suspend transmission",
/// §10.4.6.4).
pub const SUSPEND_TRANSMISSION_BITS: u64 = 8;
/// One bus-off recovery occurrence: 11 consecutive recessive bits (§12.1.4.2).
pub const RECOVERY_RECESSIVE_BITS: u64 = 11;
/// Bus-off recovery length in occurrences (ISO 11898-1:2015 §12.1.4.2 k).
pub const ISO_RECOVERY_OCCURRENCES: u32 = 128;

/// Error-warning limit (bxCAN `ESR.EWGF`, FDCAN `PSR.EW`): either counter at
/// 96 or more (RM0008 §24.7.6, RM0481 §60.4.2).
pub const ERROR_WARNING_LIMIT: u16 = 96;

/// Recent frames kept in the report; older ones are counted in
/// [`CanBusReport::frames_dropped`].
pub const FRAME_LOG_CAP: usize = 512;

/// Fewest bits from SOF to the first event a frame can cause: an error frame
/// after a mismatch. A successful frame (44 bits or more), an ACK error (at
/// least 19 bits to the ACK slot plus flag and delimiter) and an arbitration
/// loss (reported with the winner's outcome) all come later.
const MIN_EVENT_BITS: u64 =
    ERROR_DETECT_BITS + ERROR_FLAG_BITS + ERROR_ECHO_BITS + ERROR_DELIMITER_BITS;

/// Last-error codes, shared by bxCAN `ESR.LEC` and M_CAN `PSR.LEC`
/// (RM0008 §24.9.2, RM0481 §60.8.10).
pub mod lec {
    pub const NO_ERROR: u8 = 0;
    pub const STUFF: u8 = 1;
    pub const FORM: u8 = 2;
    pub const ACK: u8 = 3;
    /// Sent recessive, read dominant.
    pub const BIT_RECESSIVE: u8 = 4;
    /// Sent dominant, read recessive.
    pub const BIT_DOMINANT: u8 = 5;
    pub const CRC: u8 = 6;
}

/// A node's fault-confinement state (ISO 11898-1:2015 §12.1.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CanErrorState {
    #[default]
    Active,
    Passive,
    BusOff,
}

/// What one transmission attempt came to, from the transmitter's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanTxOutcome {
    Ok,
    /// An error frame ended the attempt (bit, stuff or form error).
    Error,
    /// No other node acknowledged the frame.
    NoAck,
    LostArbitration,
}

/// The frame a controller wants to send next.
#[derive(Debug, Clone, PartialEq)]
pub struct CanTxRequest {
    /// Controller-defined handle (bxCAN mailbox number, FDCAN buffer bit),
    /// returned in [`CanNodeUpdate::tx`].
    pub tag: u32,
    pub frame: CanFrame,
    /// The node cycle the request was made at.
    pub requested_cycle: u64,
}

/// A controller's state as the medium needs it, read at a round end.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CanNodePoll {
    /// Bit time from the controller's bit-timing register and its clock, ps.
    /// `None` when the register holds no usable timing.
    pub bit_ps: Option<u64>,
    /// The controller's kernel clock the bit time was derived from, Hz.
    pub clock_hz: u64,
    /// Clocked and in normal mode: the controller takes part in bus traffic
    /// (sends ACKs and error flags). Off in initialization, sleep, loopback.
    pub on_bus: bool,
    /// Accepts CAN-FD frames.
    pub fd_capable: bool,
    /// Highest-priority pending transmission.
    pub tx: Option<CanTxRequest>,
    /// The node cycle from which a bus-off controller may count recovery
    /// (bxCAN: normal mode after an `INRQ` cycle, or at once with `ABOM`;
    /// FDCAN: after firmware clears `CCCR.INIT`).
    pub recovery_armed_cycle: Option<u64>,
    /// Occurrences of 11 recessive bits the controller waits for to leave
    /// bus-off: [`ISO_RECOVERY_OCCURRENCES`] (bxCAN), 129 (M_CAN, RM0481).
    pub recovery_occurrences: u32,
}

/// What the medium tells one controller at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct CanNodeUpdate {
    /// The outcome of the controller's transmission attempt `(tag, outcome)`.
    pub tx: Option<(u32, CanTxOutcome)>,
    /// A frame received without error.
    pub rx: Option<CanFrame>,
    pub tec: u8,
    pub rec: u8,
    pub state: CanErrorState,
    /// New last-error code; `None` leaves it as it is.
    pub lec: Option<u8>,
}

/// An on-chip CAN controller that can take part in a timed bus. Reached
/// through [`crate::Peripheral::timed_can`] / `timed_can_mut`, so the bus
/// needs no knowledge of the concrete controller types.
pub trait TimedCanController {
    /// Enter timed mode; fails if the controller is already on a bus.
    fn attach_timed(&mut self) -> anyhow::Result<()>;
    /// The controller as the bus needs it. `apb_clock_hz` is the APB clock the
    /// bus derived for it; a controller clocked from elsewhere (the FDCAN
    /// kernel clock) ignores it.
    fn timed_poll(&self, apb_clock_hz: u64) -> CanNodePoll;
    /// Apply what the bus decided.
    fn timed_apply(&mut self, update: &CanNodeUpdate);
}

/// One frame (or one transmitter's attempt at one) in the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanFrameRecord {
    pub seq: u64,
    /// SOF time, ps.
    pub t_ps: u64,
    /// Time the attempt ended (EOF, end of error delimiter), ps.
    pub end_ps: u64,
    pub from: String,
    pub id: u32,
    pub ext: bool,
    pub rtr: bool,
    pub dlc: u8,
    /// Payload, lowercase hex.
    pub data: String,
    pub result: CanTxOutcome,
}

/// One node in the report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CanNodeReport {
    pub node: String,
    pub peripheral: String,
    /// Bit rate derived from the controller's registers, bit/s; `null` when
    /// its bit timing is unusable.
    pub bitrate: Option<u64>,
    pub clock_hz: u64,
    pub on_bus: bool,
    pub tec: u8,
    pub rec: u8,
    pub state: CanErrorState,
}

/// Frame counts by result since the bus was built.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CanBusTotals {
    pub ok: u64,
    pub error: u64,
    pub no_ack: u64,
    pub lost_arbitration: u64,
}

/// A bus's nodes, recent frames and totals.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CanBusReport {
    pub id: String,
    pub bitrate: u64,
    /// `false` for a bus left on the untimed fan-out because one of its
    /// endpoints has no timed mode; its nodes and frames are then empty.
    pub timed: bool,
    pub now_ps: u64,
    pub nodes: Vec<CanNodeReport>,
    pub frames: Vec<CanFrameRecord>,
    pub frames_dropped: u64,
    pub totals: CanBusTotals,
}

/// Bit layout of one frame on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTiming {
    /// Stuff bits inserted between SOF and the end of the CRC sequence.
    pub stuff_bits: u64,
    /// Bit index of the ACK slot, SOF = 0.
    pub ack_slot: u64,
    /// Bits from SOF through the last EOF bit.
    pub total_bits: u64,
}

/// One node's medium-side state.
#[derive(Debug, Clone)]
struct Node {
    node: String,
    peripheral: String,
    cpu_hz: u64,
    poll: Option<CanNodePoll>,
    /// Fault-confinement counters as the timeline has them. TEC is kept
    /// above 255 never: the increment that would take it past 255 puts the
    /// node bus-off and is not stored, as the 8-bit bxCAN counter shows
    /// (TEC = 0xF8 read back bus-off on an STM32F103, see `bxcan.rs`).
    tec: u16,
    rec: u16,
    state: CanErrorState,
    /// What the controller has been told so far (the report shows these).
    shown: (u8, u8, CanErrorState),
    /// A transmission attempt whose outcome the controller has not been
    /// told yet: the node takes part as a receiver only.
    in_flight: bool,
    /// Earliest time this node may start a frame (suspend transmission).
    ready_ps: u64,
    /// Bus-off recovery progress: occurrences of 11 recessive bits seen,
    /// and recessive time toward the next one.
    recovery_count: u32,
    recovery_partial_ps: u64,
}

impl Node {
    fn on_bus(&self) -> bool {
        self.state != CanErrorState::BusOff && self.poll.as_ref().is_some_and(|p| p.on_bus)
    }

    fn update_state(&mut self) {
        if self.state == CanErrorState::BusOff {
            return;
        }
        // ISO 11898-1:2015 §12.1.4.2 i: error-passive at TEC or REC > 127,
        // error-active again once both are <= 127.
        self.state = if self.tec > 127 || self.rec > 127 {
            CanErrorState::Passive
        } else {
            CanErrorState::Active
        };
    }

    /// Rule c (and d): a transmitter that sends an error flag adds 8 to TEC.
    /// Exception 1: an error-passive transmitter that sees an ACK error and no
    /// dominant bit during its passive error flag does not.
    fn tx_error(&mut self, ack_error: bool) {
        if ack_error && self.state == CanErrorState::Passive {
            return;
        }
        if self.tec + 8 > 255 {
            // Rule j: bus-off when TEC exceeds 255.
            self.state = CanErrorState::BusOff;
            self.recovery_count = 0;
            self.recovery_partial_ps = 0;
            return;
        }
        self.tec += 8;
        self.update_state();
    }

    /// Rule g: a successful transmission takes 1 from TEC.
    fn tx_ok(&mut self) {
        self.tec = self.tec.saturating_sub(1);
        self.update_state();
    }

    /// Rule a: a receiver that detects an error adds 1 to REC; rule b: 8
    /// more when the first bit after its own error flag is dominant.
    fn rx_error(&mut self, dominant_after_flag: bool) {
        let add = if dominant_after_flag { 9 } else { 1 };
        self.rec = (self.rec + add).min(255);
        self.update_state();
    }

    /// Rule h: a successful reception takes 1 from REC while it is between 1
    /// and 127; above 127 it is set back to a value between 119 and 127
    /// (the standard leaves the choice; 120 here).
    fn rx_ok(&mut self) {
        if self.rec > 127 {
            self.rec = 120;
        } else {
            self.rec = self.rec.saturating_sub(1);
        }
        self.update_state();
    }

    fn update(
        &self,
        tx: Option<(u32, CanTxOutcome)>,
        rx: Option<CanFrame>,
        lec: Option<u8>,
    ) -> CanNodeUpdate {
        CanNodeUpdate {
            tx,
            rx,
            tec: self.tec.min(255) as u8,
            rec: self.rec.min(255) as u8,
            state: self.state,
            lec,
        }
    }
}

pub struct TimedCanBus {
    id: String,
    bitrate: u64,
    bit_ps: u64,
    /// Sorted by node id, so results do not depend on manifest order.
    nodes: Vec<Node>,
    /// Earliest time the next SOF may come (after the last frame's
    /// intermission).
    bus_idle_ps: u64,
    /// Bus-off recovery has counted idle bus time up to here.
    accounted_ps: u64,
    /// The timeline is resolved up to here (last `advance_to`).
    resolved_ps: u64,
    /// Scheduled updates `(time, seq, node, update)`, in time order.
    pending: VecDeque<(u64, u64, usize, CanNodeUpdate)>,
    next_seq: u64,
    frames: VecDeque<CanFrameRecord>,
    frames_dropped: u64,
    next_frame_seq: u64,
    totals: CanBusTotals,
}

impl TimedCanBus {
    /// A bus `id` at `bitrate` bit/s with `nodes` as `(node id, controller
    /// id, node CPU clock Hz)`, in any order.
    pub fn new(id: &str, bitrate: u64, nodes: Vec<(String, String, u64)>) -> Self {
        let bitrate = bitrate.max(1);
        let mut nodes: Vec<Node> = nodes
            .into_iter()
            .map(|(node, peripheral, cpu_hz)| Node {
                node,
                peripheral,
                cpu_hz,
                poll: None,
                tec: 0,
                rec: 0,
                state: CanErrorState::Active,
                shown: (0, 0, CanErrorState::Active),
                in_flight: false,
                ready_ps: 0,
                recovery_count: 0,
                recovery_partial_ps: 0,
            })
            .collect();
        nodes.sort_by(|a, b| a.node.cmp(&b.node));
        Self {
            id: id.to_string(),
            bitrate,
            bit_ps: (PS_PER_SEC / u128::from(bitrate)) as u64,
            nodes,
            bus_idle_ps: 0,
            accounted_ps: 0,
            resolved_ps: 0,
            pending: VecDeque::new(),
            next_seq: 0,
            frames: VecDeque::new(),
            frames_dropped: 0,
            next_frame_seq: 0,
            totals: CanBusTotals::default(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// One bus bit, ps.
    pub fn bit_ps(&self) -> u64 {
        self.bit_ps
    }

    /// The shortest time from a node starting a frame to any update the
    /// frame causes. A world whose rounds are no longer than this learns of
    /// every update before it is due.
    pub fn lookahead_ps(&self) -> u64 {
        MIN_EVENT_BITS * self.bit_ps
    }

    /// Time of the next scheduled update, or of a bus-off recovery that
    /// completes if the bus stays idle.
    pub fn next_event_ps(&self) -> Option<u64> {
        let pending = self.pending.front().map(|(t, ..)| *t);
        let recovery = (0..self.nodes.len())
            .filter_map(|i| self.recovery_due_ps(i))
            .min();
        match (pending, recovery) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// `(node id, controller id)` of node `idx`.
    pub fn node(&self, idx: usize) -> (&str, &str) {
        let n = &self.nodes[idx];
        (&n.node, &n.peripheral)
    }

    /// Record node `idx`'s controller state; `None` when it could not be read.
    pub fn set_poll(&mut self, idx: usize, poll: Option<CanNodePoll>) {
        self.nodes[idx].poll = poll;
    }

    /// The updates due at or before `now_ps`, in time order (node order at
    /// equal times).
    pub fn take_due(&mut self, now_ps: u64) -> Vec<(usize, CanNodeUpdate)> {
        let mut out = Vec::new();
        while self.pending.front().is_some_and(|(t, ..)| *t <= now_ps) {
            let (_, _, idx, update) = self.pending.pop_front().expect("checked");
            let node = &mut self.nodes[idx];
            if update.tx.is_some() {
                node.in_flight = false;
            }
            node.shown = (update.tec, update.rec, update.state);
            out.push((idx, update));
        }
        out
    }

    /// Resolve the bus up to world time `r_ps`, which every node has reached:
    /// start the frames due by then and schedule what they cause.
    pub fn advance_to(&mut self, r_ps: u64) {
        let floor = self.resolved_ps;
        loop {
            let slot = self.next_slot(floor).filter(|(s, _)| *s <= r_ps);
            let horizon = slot.as_ref().map_or(r_ps, |(s, _)| *s);
            self.account_idle(horizon);
            let Some((start, contenders)) = slot else {
                break;
            };
            self.run_slot(start, &contenders);
        }
        self.resolved_ps = self.resolved_ps.max(r_ps);
    }

    fn matched(&self, idx: usize) -> bool {
        let Some(bit) = self.nodes[idx].poll.as_ref().and_then(|p| p.bit_ps) else {
            return false;
        };
        let bus = self.bit_ps as f64;
        ((bit as f64 - bus) / bus).abs() <= BITRATE_TOLERANCE
    }

    fn request_ps(&self, idx: usize, floor: u64) -> Option<u64> {
        let n = &self.nodes[idx];
        if n.in_flight || !n.on_bus() {
            return None;
        }
        let tx = n.poll.as_ref()?.tx.as_ref()?;
        Some(
            cycles_to_ps(tx.requested_cycle, n.cpu_hz)
                .max(floor)
                .max(n.ready_ps),
        )
    }

    /// The next frame start and the nodes that contend for it: every node
    /// with a request pending at that instant. A request that comes while a
    /// frame is on the bus waits for the bus to go idle.
    fn next_slot(&self, floor: u64) -> Option<(u64, Vec<usize>)> {
        let ready: Vec<(usize, u64)> = (0..self.nodes.len())
            .filter_map(|i| self.request_ps(i, floor).map(|t| (i, t)))
            .collect();
        let earliest = ready.iter().map(|(_, t)| *t).min()?;
        let start = earliest.max(self.bus_idle_ps);
        let contenders = ready
            .iter()
            .filter(|(_, t)| *t <= start)
            .map(|(i, _)| *i)
            .collect();
        Some((start, contenders))
    }

    fn tx_frame(&self, idx: usize) -> (u32, CanFrame) {
        let tx = self.nodes[idx]
            .poll
            .as_ref()
            .and_then(|p| p.tx.as_ref())
            .expect("a contender has a request");
        (tx.tag, tx.frame.clone())
    }

    /// Run the frame (or collision) that starts at `start`.
    fn run_slot(&mut self, start: u64, contenders: &[usize]) {
        let bit = self.bit_ps;
        let receivers: Vec<usize> = (0..self.nodes.len())
            .filter(|i| !contenders.contains(i) && self.nodes[*i].on_bus())
            .collect();
        let mut updates: Vec<Option<CanNodeUpdate>> = vec![None; self.nodes.len()];
        let mut rows: Vec<(usize, CanTxOutcome)> = Vec::new();
        let error_end = start + MIN_EVENT_BITS * bit;

        let mismatched: Vec<usize> = contenders
            .iter()
            .copied()
            .filter(|i| !self.matched(*i))
            .collect();
        let end = if !mismatched.is_empty() {
            // A transmitter on the wrong bit rate garbles the arbitration
            // field. A correct-rate contender that reads a dominant level it
            // did not send there has lost arbitration, not seen a bit error
            // (ISO 11898-1 §10.4.4: no bit error in the arbitration field),
            // and the frame is then destroyed by the receivers' error flags.
            for &i in contenders {
                let (tag, _) = self.tx_frame(i);
                if mismatched.contains(&i) {
                    self.nodes[i].tx_error(false);
                    updates[i] = Some(self.nodes[i].update(
                        Some((tag, CanTxOutcome::Error)),
                        None,
                        Some(lec::BIT_RECESSIVE),
                    ));
                    rows.push((i, CanTxOutcome::Error));
                } else {
                    self.nodes[i].rx_error(false);
                    updates[i] = Some(self.nodes[i].update(
                        Some((tag, CanTxOutcome::LostArbitration)),
                        None,
                        Some(lec::STUFF),
                    ));
                    rows.push((i, CanTxOutcome::LostArbitration));
                }
            }
            for &i in &receivers {
                self.nodes[i].rx_error(false);
                updates[i] = Some(self.nodes[i].update(None, None, Some(lec::STUFF)));
            }
            error_end
        } else {
            let mut order: Vec<(u64, usize)> = contenders
                .iter()
                .map(|&i| (arbitration_key(&self.tx_frame(i).1), i))
                .collect();
            order.sort();
            let best = order[0].0;
            let tied: Vec<usize> = order
                .iter()
                .filter(|(k, _)| *k == best)
                .map(|(_, i)| *i)
                .collect();
            let losers: Vec<usize> = order
                .iter()
                .filter(|(k, _)| *k != best)
                .map(|(_, i)| *i)
                .collect();
            let frame = self.tx_frame(tied[0]).1;
            // Receivers of the winning frame: the bus nodes and the losers.
            let mut listeners = receivers.clone();
            listeners.extend(losers.iter().copied());
            listeners.sort_unstable();
            let identical = tied.iter().all(|&i| self.tx_frame(i).1 == frame);
            let end = if !identical {
                // Two transmitters won arbitration with the same identifier
                // but different bits: the one sending recessive reads
                // dominant and flags a bit error, destroying the frame.
                for &i in &tied {
                    let tag = self.tx_frame(i).0;
                    self.nodes[i].tx_error(false);
                    updates[i] = Some(self.nodes[i].update(
                        Some((tag, CanTxOutcome::Error)),
                        None,
                        Some(lec::BIT_RECESSIVE),
                    ));
                    rows.push((i, CanTxOutcome::Error));
                }
                for &i in &listeners {
                    self.nodes[i].rx_error(false);
                    updates[i] = Some(self.nodes[i].update(None, None, Some(lec::STUFF)));
                }
                error_end
            } else {
                self.run_frame(start, &tied, &listeners, &frame, &mut updates, &mut rows)
            };
            for &i in &losers {
                let tag = self.tx_frame(i).0;
                let prior = updates[i].take();
                let (rx, lec) = prior.map_or((None, None), |u| (u.rx, u.lec));
                updates[i] =
                    Some(self.nodes[i].update(Some((tag, CanTxOutcome::LostArbitration)), rx, lec));
                rows.push((i, CanTxOutcome::LostArbitration));
            }
            end
        };

        let idle = end + INTERMISSION_BITS * bit;
        for &i in contenders {
            self.nodes[i].in_flight = true;
            if self.nodes[i].state == CanErrorState::Passive
                && rows
                    .iter()
                    .any(|(r, o)| *r == i && *o != CanTxOutcome::LostArbitration)
            {
                self.nodes[i].ready_ps = idle + SUSPEND_TRANSMISSION_BITS * bit;
            }
        }
        // The frame or error frame ends in 11 recessive bits (ACK delimiter
        // + EOF + intermission, or error delimiter + intermission): one
        // recovery occurrence for each node counting toward leaving bus-off.
        for i in 0..self.nodes.len() {
            if self.recovery_armed_ps(i).is_some_and(|a| a <= start) {
                self.nodes[i].recovery_count += 1;
                self.nodes[i].recovery_partial_ps = 0;
                self.maybe_recover(i, idle);
            } else {
                self.nodes[i].recovery_partial_ps = 0;
            }
        }
        for (i, row) in rows {
            let frame = self.tx_frame(i).1;
            self.log_frame(start, end, i, &frame, row);
        }
        for (i, u) in updates.into_iter().enumerate() {
            if let Some(u) = u {
                self.schedule(end, i, u);
            }
        }
        self.bus_idle_ps = idle;
        self.accounted_ps = self.accounted_ps.max(idle);
    }

    /// The winning frame on the bus: destroyed by an error-active receiver
    /// that cannot sample it, unacknowledged, or received.
    fn run_frame(
        &mut self,
        start: u64,
        transmitters: &[usize],
        listeners: &[usize],
        frame: &CanFrame,
        updates: &mut [Option<CanNodeUpdate>],
        rows: &mut Vec<(usize, CanTxOutcome)>,
    ) -> u64 {
        let bit = self.bit_ps;
        let timing = frame_timing(frame);
        let destroyers: Vec<usize> = listeners
            .iter()
            .copied()
            .filter(|&i| !self.matched(i) && self.nodes[i].state == CanErrorState::Active)
            .collect();
        let correct: Vec<usize> = listeners
            .iter()
            .copied()
            .filter(|&i| self.matched(i))
            .collect();
        if !destroyers.is_empty() {
            // The mismatched receiver's active error flag destroys the frame.
            // The transmitter reads dominant where it sent recessive (rule
            // c); the other receivers see six dominant bits, a stuff error
            // (rule a); the destroyer then reads the others' flags as
            // dominant bits right after its own (rule b).
            for &i in transmitters {
                let tag = self.tx_frame(i).0;
                self.nodes[i].tx_error(false);
                updates[i] = Some(self.nodes[i].update(
                    Some((tag, CanTxOutcome::Error)),
                    None,
                    Some(lec::BIT_RECESSIVE),
                ));
                rows.push((i, CanTxOutcome::Error));
            }
            for &i in listeners {
                self.nodes[i].rx_error(destroyers.contains(&i));
                updates[i] = Some(self.nodes[i].update(None, None, Some(lec::STUFF)));
            }
            return start + MIN_EVENT_BITS * bit;
        }
        if correct.is_empty() {
            // Nobody drives the ACK slot dominant (ISO 11898-1 §10.11): the
            // transmitter flags an ACK error from the bit after the slot.
            for &i in transmitters {
                let tag = self.tx_frame(i).0;
                self.nodes[i].tx_error(true);
                updates[i] = Some(self.nodes[i].update(
                    Some((tag, CanTxOutcome::NoAck)),
                    None,
                    Some(lec::ACK),
                ));
                rows.push((i, CanTxOutcome::NoAck));
            }
            for &i in listeners {
                // Error-passive and on the wrong bit rate: it detects an
                // error, but its passive flag disturbs nobody.
                self.nodes[i].rx_error(false);
                updates[i] = Some(self.nodes[i].update(None, None, Some(lec::STUFF)));
            }
            return start + (timing.ack_slot + 1 + ERROR_FLAG_BITS + ERROR_DELIMITER_BITS) * bit;
        }
        for &i in transmitters {
            let tag = self.tx_frame(i).0;
            self.nodes[i].tx_ok();
            updates[i] = Some(self.nodes[i].update(
                Some((tag, CanTxOutcome::Ok)),
                None,
                Some(lec::NO_ERROR),
            ));
            rows.push((i, CanTxOutcome::Ok));
        }
        for &i in listeners {
            if correct.contains(&i) {
                self.nodes[i].rx_ok();
                // A classic controller does not store a CAN-FD frame.
                let fd_ok = !frame.fd || self.nodes[i].poll.as_ref().is_some_and(|p| p.fd_capable);
                let rx = fd_ok.then(|| frame.clone());
                updates[i] = Some(self.nodes[i].update(None, rx, Some(lec::NO_ERROR)));
            } else {
                self.nodes[i].rx_error(false);
                updates[i] = Some(self.nodes[i].update(None, None, Some(lec::STUFF)));
            }
        }
        start + timing.total_bits * bit
    }

    fn schedule(&mut self, at: u64, idx: usize, update: CanNodeUpdate) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let pos = self
            .pending
            .partition_point(|(t, s, i, _)| (*t, *i, *s) <= (at, idx, seq));
        self.pending.insert(pos, (at, seq, idx, update));
    }

    fn log_frame(
        &mut self,
        start: u64,
        end: u64,
        idx: usize,
        frame: &CanFrame,
        result: CanTxOutcome,
    ) {
        match result {
            CanTxOutcome::Ok => self.totals.ok += 1,
            CanTxOutcome::Error => self.totals.error += 1,
            CanTxOutcome::NoAck => self.totals.no_ack += 1,
            CanTxOutcome::LostArbitration => self.totals.lost_arbitration += 1,
        }
        if self.frames.len() == FRAME_LOG_CAP {
            self.frames.pop_front();
            self.frames_dropped += 1;
        }
        let data = if frame.remote {
            String::new()
        } else {
            frame.data.iter().map(|b| format!("{b:02x}")).collect()
        };
        self.frames.push_back(CanFrameRecord {
            seq: self.next_frame_seq,
            t_ps: start,
            end_ps: end,
            from: self.nodes[idx].node.clone(),
            id: frame.id,
            ext: frame.extended,
            rtr: frame.remote,
            dlc: len_to_dlc(frame.data.len()),
            data,
            result,
        });
        self.next_frame_seq += 1;
    }

    /// The world time from which bus-off node `idx` counts recovery.
    fn recovery_armed_ps(&self, idx: usize) -> Option<u64> {
        let n = &self.nodes[idx];
        if n.state != CanErrorState::BusOff {
            return None;
        }
        let cycle = n.poll.as_ref()?.recovery_armed_cycle?;
        Some(cycles_to_ps(cycle, n.cpu_hz))
    }

    fn recovery_needed(&self, idx: usize) -> u32 {
        self.nodes[idx]
            .poll
            .as_ref()
            .map_or(ISO_RECOVERY_OCCURRENCES, |p| p.recovery_occurrences.max(1))
    }

    /// When node `idx` leaves bus-off if the bus stays idle from now on.
    fn recovery_due_ps(&self, idx: usize) -> Option<u64> {
        let armed = self.recovery_armed_ps(idx)?;
        let n = &self.nodes[idx];
        let from = self.accounted_ps.max(self.bus_idle_ps).max(armed);
        let left = u64::from(self.recovery_needed(idx).saturating_sub(n.recovery_count));
        let span =
            (left * RECOVERY_RECESSIVE_BITS * self.bit_ps).saturating_sub(n.recovery_partial_ps);
        Some(from + span)
    }

    /// Count idle bus time up to `to` toward bus-off recovery.
    fn account_idle(&mut self, to: u64) {
        let from = self.accounted_ps.max(self.bus_idle_ps);
        if to <= from {
            return;
        }
        let occurrence = RECOVERY_RECESSIVE_BITS * self.bit_ps;
        for i in 0..self.nodes.len() {
            let Some(armed) = self.recovery_armed_ps(i) else {
                continue;
            };
            let begin = from.max(armed);
            if to <= begin {
                continue;
            }
            let needed = self.recovery_needed(i);
            let n = &mut self.nodes[i];
            let run = n.recovery_partial_ps + (to - begin);
            let left = needed.saturating_sub(n.recovery_count);
            if run / occurrence >= u64::from(left) {
                let at = begin + u64::from(left) * occurrence - n.recovery_partial_ps;
                n.recovery_count = needed;
                n.recovery_partial_ps = 0;
                self.maybe_recover(i, at);
            } else {
                n.recovery_count += (run / occurrence) as u32;
                n.recovery_partial_ps = run % occurrence;
            }
        }
        self.accounted_ps = to;
    }

    /// Rule k: after the recovery sequence a bus-off node is error-active
    /// with both counters at 0.
    fn maybe_recover(&mut self, idx: usize, at: u64) {
        if self.nodes[idx].recovery_count < self.recovery_needed(idx) {
            return;
        }
        let n = &mut self.nodes[idx];
        n.state = CanErrorState::Active;
        n.tec = 0;
        n.rec = 0;
        n.recovery_count = 0;
        n.recovery_partial_ps = 0;
        let update = n.update(None, None, None);
        self.schedule(at, idx, update);
    }

    /// The bus's nodes as the controllers see them, the recent frames and
    /// the totals.
    pub fn report(&self, now_ps: u64) -> CanBusReport {
        CanBusReport {
            id: self.id.clone(),
            bitrate: self.bitrate,
            timed: true,
            now_ps,
            nodes: self
                .nodes
                .iter()
                .map(|n| {
                    let poll = n.poll.as_ref();
                    CanNodeReport {
                        node: n.node.clone(),
                        peripheral: n.peripheral.clone(),
                        bitrate: poll
                            .and_then(|p| p.bit_ps)
                            .filter(|b| *b > 0)
                            .map(|b| ((PS_PER_SEC as f64) / b as f64).round() as u64),
                        clock_hz: poll.map_or(0, |p| p.clock_hz),
                        on_bus: poll.is_some_and(|p| p.on_bus),
                        tec: n.shown.0,
                        rec: n.shown.1,
                        state: n.shown.2,
                    }
                })
                .collect(),
            frames: self.frames.iter().cloned().collect(),
            frames_dropped: self.frames_dropped,
            totals: self.totals.clone(),
        }
    }
}

/// The frame's layout on the wire: the real stuff bits of its real bits.
///
/// A CAN-FD frame is laid out as a classic frame with the same payload at the
/// nominal bit rate: the FD control bits, the CRC-17/21 and the data-phase
/// bit rate are not modelled, so an FD frame takes longer here than on a bus
/// that switches bit rate.
pub fn frame_timing(frame: &CanFrame) -> FrameTiming {
    let mut bits: Vec<bool> = Vec::with_capacity(160);
    let push = |bits: &mut Vec<bool>, value: u32, n: u32| {
        for k in (0..n).rev() {
            bits.push((value >> k) & 1 == 1);
        }
    };
    bits.push(false); // SOF
    if frame.extended {
        push(&mut bits, (frame.id >> 18) & 0x7FF, 11);
        bits.push(true); // SRR
        bits.push(true); // IDE
        push(&mut bits, frame.id & 0x3FFFF, 18);
        bits.push(frame.remote);
        bits.push(false); // r1
        bits.push(false); // r0
    } else {
        push(&mut bits, frame.id & 0x7FF, 11);
        bits.push(frame.remote);
        bits.push(false); // IDE
        bits.push(false); // r0
    }
    push(&mut bits, u32::from(len_to_dlc(frame.data.len())), 4);
    if !frame.remote {
        for byte in &frame.data {
            push(&mut bits, u32::from(*byte), 8);
        }
    }
    let crc = crc15(&bits);
    push(&mut bits, u32::from(crc), 15);
    let stuff_bits = stuff_count(&bits);
    let through_crc = bits.len() as u64 + stuff_bits;
    FrameTiming {
        stuff_bits,
        // CRC delimiter, then the ACK slot.
        ack_slot: through_crc + 1,
        // CRC delimiter, ACK slot, ACK delimiter, 7 EOF bits.
        total_bits: through_crc + 10,
    }
}

/// CRC-15/CAN, polynomial x^15+x^14+x^10+x^8+x^7+x^4+x^3+1 (0x4599), over
/// SOF through the data field (ISO 11898-1:2015 §10.4.2.6).
fn crc15(bits: &[bool]) -> u16 {
    let mut crc: u16 = 0;
    for &bit in bits {
        let next = bit ^ ((crc >> 14) & 1 == 1);
        crc = (crc << 1) & 0x7FFF;
        if next {
            crc ^= 0x4599;
        }
    }
    crc
}

/// Stuff bits a transmitter inserts: after five equal bits, one of the
/// opposite level, which itself starts the next run (§10.5).
fn stuff_count(bits: &[bool]) -> u64 {
    let mut count = 0;
    let mut run = 0;
    let mut last = None;
    for &bit in bits {
        if last == Some(bit) {
            run += 1;
        } else {
            run = 1;
            last = Some(bit);
        }
        if run == 5 {
            count += 1;
            last = Some(!bit);
            run = 1;
        }
    }
    count
}

/// The frame's arbitration field as a number that orders like the wire:
/// smaller wins. Standard: ID[10:0], RTR, IDE=0. Extended: ID[28:18],
/// SRR=1, IDE=1, ID[17:0], RTR (ISO 11898-1:2015 §10.4.2.2).
pub fn arbitration_key(frame: &CanFrame) -> u64 {
    let rtr = u64::from(frame.remote);
    if frame.extended {
        let id = u64::from(frame.id & 0x1FFF_FFFF);
        ((id >> 18) << 21) | (1 << 20) | (1 << 19) | ((id & 0x3FFFF) << 1) | rtr
    } else {
        (u64::from(frame.id & 0x7FF) << 21) | (rtr << 20)
    }
}

fn len_to_dlc(len: usize) -> u8 {
    match len {
        0..=8 => len as u8,
        9..=12 => 9,
        13..=16 => 10,
        17..=20 => 11,
        21..=24 => 12,
        25..=32 => 13,
        33..=48 => 14,
        _ => 15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ: u64 = 72_000_000;
    const BIT_500K: u64 = 2_000_000; // ps
    const BIT_250K: u64 = 4_000_000; // ps

    /// A stand-in controller: a FIFO of requests, auto-retransmission unless
    /// the update says the request is done, and the frames it received.
    #[derive(Default)]
    struct FakeNode {
        bit_ps: u64,
        on_bus: bool,
        queue: VecDeque<(u32, CanFrame, u64)>,
        next_tag: u32,
        rx: Vec<(u64, CanFrame)>,
        outcomes: Vec<(u64, CanTxOutcome)>,
        state: CanErrorState,
        tec: u8,
        rec: u8,
        armed: Option<u64>,
        /// Re-arm recovery at once after bus-off (bxCAN ABOM).
        auto_recover: bool,
    }

    impl FakeNode {
        fn new(bit_ps: u64) -> Self {
            Self {
                bit_ps,
                on_bus: true,
                ..Default::default()
            }
        }
        fn send(&mut self, frame: CanFrame, at_cycle: u64) {
            self.next_tag += 1;
            self.queue.push_back((self.next_tag, frame, at_cycle));
        }
        fn poll(&self) -> CanNodePoll {
            CanNodePoll {
                bit_ps: Some(self.bit_ps),
                clock_hz: 36_000_000,
                on_bus: self.on_bus,
                fd_capable: false,
                tx: self.queue.front().map(|(tag, f, c)| CanTxRequest {
                    tag: *tag,
                    frame: f.clone(),
                    requested_cycle: *c,
                }),
                recovery_armed_cycle: self.armed,
                recovery_occurrences: ISO_RECOVERY_OCCURRENCES,
            }
        }
        fn apply(&mut self, now_ps: u64, u: CanNodeUpdate) {
            if let Some((tag, outcome)) = u.tx {
                self.outcomes.push((now_ps, outcome));
                if outcome == CanTxOutcome::Ok {
                    assert_eq!(self.queue.front().map(|q| q.0), Some(tag));
                    self.queue.pop_front();
                }
            }
            if let Some(f) = u.rx {
                self.rx.push((now_ps, f));
            }
            if u.state == CanErrorState::BusOff && self.state != CanErrorState::BusOff {
                self.armed = self.auto_recover.then(|| ps_to_cycles(now_ps));
            }
            if u.state != CanErrorState::BusOff {
                self.armed = None;
            }
            self.state = u.state;
            self.tec = u.tec;
            self.rec = u.rec;
        }
    }

    fn ps_to_cycles(ps: u64) -> u64 {
        crate::network::timed_uart::ps_to_cycles_ceil(ps, HZ)
    }

    /// A world in miniature: rounds bounded by the lookahead and the next
    /// event, updates applied at round starts, polls at round ends.
    struct Harness {
        bus: TimedCanBus,
        nodes: Vec<FakeNode>,
        now: u64,
    }

    impl Harness {
        fn new(nodes: Vec<(&str, FakeNode)>) -> Self {
            let bus = TimedCanBus::new(
                "can0",
                500_000,
                nodes
                    .iter()
                    .map(|(id, _)| (id.to_string(), "can1".to_string(), HZ))
                    .collect(),
            );
            // The bus sorts its nodes; keep ours in the same order.
            let mut nodes: Vec<(&str, FakeNode)> = nodes;
            nodes.sort_by(|a, b| a.0.cmp(b.0));
            Self {
                bus,
                nodes: nodes.into_iter().map(|(_, n)| n).collect(),
                now: 0,
            }
        }
        fn idx(&self, id: &str) -> usize {
            (0..self.bus.node_count())
                .find(|&i| self.bus.node(i).0 == id)
                .unwrap()
        }
        fn node(&mut self, id: &str) -> &mut FakeNode {
            let i = self.idx(id);
            &mut self.nodes[i]
        }
        /// One round. `firmware` runs at the round start (it may queue
        /// requests stamped inside the round).
        fn round(&mut self) {
            for (i, u) in self.bus.take_due(self.now) {
                self.nodes[i].apply(self.now, u);
            }
            let mut end = self.now + self.bus.lookahead_ps();
            if let Some(t) = self.bus.next_event_ps() {
                if t > self.now {
                    end = end.min(t);
                }
            }
            self.now = end;
            for i in 0..self.nodes.len() {
                self.bus.set_poll(i, Some(self.nodes[i].poll()));
            }
            self.bus.advance_to(self.now);
        }
        /// Run until every node has reached `t`, and apply what is due by
        /// then (not what the last round's end, which may be later, has).
        fn run_until(&mut self, t: u64) {
            while self.now < t {
                self.round();
            }
            let at = t.min(self.now);
            for (i, u) in self.bus.take_due(at) {
                self.nodes[i].apply(at, u);
            }
        }
    }

    fn std(id: u32, data: &[u8]) -> CanFrame {
        CanFrame::classic(id, data.to_vec())
    }

    fn ext(id: u32, data: &[u8]) -> CanFrame {
        CanFrame {
            extended: true,
            ..CanFrame::classic(id, data.to_vec())
        }
    }

    #[test]
    fn frame_length_counts_every_field_and_the_real_stuff_bits() {
        // Standard data frame: 44 fixed bits + 8 per data byte, plus stuff.
        // Extended: 64 + 8 per byte. Stuff counts below are independently
        // computed (CRC-15 polynomial 0x4599 over SOF..data, a stuff bit
        // after every five equal bits through the CRC sequence).
        let t = frame_timing(&std(0x555, &[]));
        assert_eq!(t.stuff_bits, 1);
        assert_eq!(t.total_bits, 44 + t.stuff_bits);
        assert_eq!(t.ack_slot, t.total_bits - 9, "ACK slot, delimiter, 7 EOF");
        let t = frame_timing(&std(0x000, &[]));
        assert_eq!(t.stuff_bits, 6, "34 zero bits stuff six times");
        let t = frame_timing(&std(0x100, &[0x0F, 0xA0]));
        assert_eq!(t.total_bits, 44 + 16 + t.stuff_bits);
        assert_eq!(t.stuff_bits, 6);
        let t = frame_timing(&ext(0x18FEF100, &[0xFF; 8]));
        assert_eq!(t.total_bits, 64 + 64 + t.stuff_bits);
        assert_eq!(t.stuff_bits, 15);
        // Never more than the worst case: one stuff bit per four bits after
        // the first five.
        for id in [0u32, 0x7FF, 0x2AA, 0x123] {
            for len in 0..=8 {
                let f = std(id, &vec![0u8; len]);
                let region = 34 + 8 * len as u64 + 15;
                assert!(frame_timing(&f).stuff_bits <= (region - 1) / 4);
            }
        }
    }

    #[test]
    fn arbitration_key_orders_like_the_wire() {
        // Lower identifier wins.
        assert!(arbitration_key(&std(0x100, &[])) < arbitration_key(&std(0x200, &[])));
        // Standard beats extended at the same base identifier (SRR, IDE
        // recessive), whatever the extended bits.
        assert!(arbitration_key(&std(0x100, &[])) < arbitration_key(&ext(0x100 << 18, &[])));
        let remote = CanFrame {
            remote: true,
            ..std(0x100, &[])
        };
        assert!(arbitration_key(&remote) < arbitration_key(&ext(0x100 << 18, &[])));
        // Data beats remote at the same identifier.
        assert!(arbitration_key(&std(0x100, &[])) < arbitration_key(&remote));
        // A lower base identifier beats any frame with a higher one.
        assert!(
            arbitration_key(&ext(0x0FF << 18 | 0x3FFFF, &[])) < arbitration_key(&std(0x100, &[]))
        );
    }

    #[test]
    fn a_frame_arrives_at_its_eof_and_not_a_bit_earlier() {
        let mut h = Harness::new(vec![
            ("dash", FakeNode::new(BIT_500K)),
            ("engine", FakeNode::new(BIT_500K)),
        ]);
        let frame = std(0x100, &[0x0F, 0xA0]);
        // Requested at cycle 72 = 1 µs.
        h.node("engine").send(frame.clone(), 72);
        let bits = frame_timing(&frame).total_bits;
        let eof = 1_000_000 + bits * BIT_500K;
        h.run_until(eof - 1);
        assert!(h.node("dash").rx.is_empty(), "nothing before the EOF");
        h.run_until(eof);
        assert_eq!(h.node("dash").rx, vec![(eof, frame)]);
        assert_eq!(h.node("engine").outcomes, vec![(eof, CanTxOutcome::Ok)]);
        let report = h.bus.report(h.now);
        assert_eq!(report.frames.len(), 1);
        assert_eq!(report.frames[0].t_ps, 1_000_000);
        assert_eq!(report.frames[0].end_ps, eof);
        assert_eq!(report.frames[0].data, "0fa0");
        assert_eq!(report.frames[0].dlc, 2);
        assert_eq!(report.totals.ok, 1);
    }

    #[test]
    fn lowest_id_wins_arbitration_and_the_loser_retries_after_intermission() {
        let mut h = Harness::new(vec![
            ("a", FakeNode::new(BIT_500K)),
            ("b", FakeNode::new(BIT_500K)),
            ("c", FakeNode::new(BIT_500K)),
        ]);
        let hi = std(0x200, &[1]);
        let lo = std(0x100, &[2]);
        h.node("a").send(hi.clone(), 0);
        h.node("b").send(lo.clone(), 0);
        h.run_until(2_000_000_000);
        let first_end = frame_timing(&lo).total_bits * BIT_500K;
        let second_start = first_end + INTERMISSION_BITS * BIT_500K;
        let second_end = second_start + frame_timing(&hi).total_bits * BIT_500K;
        assert_eq!(
            h.node("c").rx,
            vec![(first_end, lo.clone()), (second_end, hi.clone())]
        );
        assert_eq!(
            h.node("a").outcomes,
            vec![
                (first_end, CanTxOutcome::LostArbitration),
                (second_end, CanTxOutcome::Ok)
            ]
        );
        // The loser receives the winner's frame.
        assert_eq!(h.node("a").rx, vec![(first_end, lo)]);
        let report = h.bus.report(h.now);
        let results: Vec<_> = report
            .frames
            .iter()
            .map(|f| (f.from.as_str(), f.result))
            .collect();
        assert_eq!(
            results,
            vec![
                ("b", CanTxOutcome::Ok),
                ("a", CanTxOutcome::LostArbitration),
                ("a", CanTxOutcome::Ok)
            ]
        );
        assert_eq!(report.totals.lost_arbitration, 1);
    }

    #[test]
    fn standard_frame_beats_extended_with_the_same_base_id() {
        let mut h = Harness::new(vec![
            ("a", FakeNode::new(BIT_500K)),
            ("b", FakeNode::new(BIT_500K)),
            ("c", FakeNode::new(BIT_500K)),
        ]);
        h.node("a").send(ext(0x100 << 18, &[]), 0);
        h.node("b").send(std(0x100, &[]), 0);
        h.run_until(1_000_000_000);
        let ids: Vec<bool> = h.node("c").rx.iter().map(|(_, f)| f.extended).collect();
        assert_eq!(ids, vec![false, true]);
    }

    #[test]
    fn a_frame_nobody_acknowledges_is_a_transmit_error_and_stops_at_error_passive() {
        let mut quiet = FakeNode::new(BIT_500K);
        quiet.on_bus = false; // in initialization: no ACK
        let mut h = Harness::new(vec![("lone", FakeNode::new(BIT_500K)), ("quiet", quiet)]);
        let frame = std(0x123, &[0xAA]);
        h.node("lone").send(frame.clone(), 0);
        let t = frame_timing(&frame);
        let first_err = (t.ack_slot + 1 + ERROR_FLAG_BITS + ERROR_DELIMITER_BITS) * BIT_500K;
        h.run_until(first_err);
        assert_eq!(
            h.node("lone").outcomes,
            vec![(first_err, CanTxOutcome::NoAck)]
        );
        assert_eq!(h.node("lone").tec, 8, "TEC += 8 (ISO 11898-1 §12.1.4.2 c)");
        h.run_until(50_000_000_000);
        let lone = h.node("lone");
        // 16 errors take it to 128 = error-passive; then the passive-ACK
        // exception holds TEC there: a lone node never goes bus-off.
        assert_eq!(lone.tec, 128);
        assert_eq!(lone.state, CanErrorState::Passive);
        assert!(lone.outcomes.iter().all(|(_, o)| *o == CanTxOutcome::NoAck));
        assert!(lone.outcomes.len() > 20, "it keeps retrying");
        assert_eq!(h.bus.report(h.now).totals.ok, 0);
    }

    /// The promo story: an engine sends 0x100 every 10 ms, a dashboard
    /// listens, and a new sensor on 250 kbit/s joins a 500 kbit/s bus and
    /// sends 0x200 every 100 ms.
    fn story(order: &[&str]) -> Harness {
        let mut nodes = Vec::new();
        for id in order {
            let mut n = FakeNode::new(if *id == "sensor" { BIT_250K } else { BIT_500K });
            n.auto_recover = *id == "sensor";
            nodes.push((*id, n));
        }
        Harness::new(nodes)
    }

    const MS: u64 = 1_000_000_000;

    /// Drive the story's firmware up to `until_ps`; the sensor's
    /// `(time, TEC, REC, state)` after every round.
    fn drive_story(
        h: &mut Harness,
        until_ps: u64,
    ) -> Vec<(u64, u8, u8, CanErrorState, u8, CanErrorState)> {
        let mut sensor_track = Vec::new();
        let ms = MS;
        let mut next_engine = h.now.div_ceil(10 * ms) * 10 * ms;
        // Every 100 ms from 5 ms on: the first such time not yet passed.
        let mut next_sensor = (h.now + 95 * ms) / (100 * ms) * (100 * ms) + 5 * ms;
        while h.now < until_ps {
            if h.now >= next_engine {
                let c = ps_to_cycles(next_engine);
                let rpm = (next_engine / ms) as u16;
                h.node("engine").send(std(0x100, &rpm.to_be_bytes()), c);
                next_engine += 10 * ms;
            }
            if h.now >= next_sensor {
                let c = ps_to_cycles(next_sensor);
                h.node("sensor").send(std(0x200, &[42]), c);
                next_sensor += 100 * ms;
            }
            h.round();
            let now = h.now;
            let (etec, estate) = (h.node("engine").tec, h.node("engine").state);
            let s = h.node("sensor");
            sensor_track.push((now, s.tec, s.rec, s.state, etec, estate));
        }
        sensor_track
    }

    #[test]
    fn a_wrong_bitrate_node_destroys_traffic_then_goes_passive_then_bus_off_then_recovers() {
        let mut h = story(&["dash", "engine", "sensor"]);
        // Before the sensor sends anything, it is an error-active receiver
        // that cannot sample the engine's frames: it destroys them, and the
        // engine retransmits into the same fate.
        let mut track = drive_story(&mut h, MS / 2);
        let rep = h.bus.report(h.now);
        assert!(rep.frames.len() > 3);
        assert!(rep.frames.iter().all(|f| f.result == CanTxOutcome::Error));
        assert!(h.node("dash").rx.is_empty(), "no engine frame got through");
        assert!(h.node("engine").tec >= 24, "TEC += 8 per destroyed frame");
        assert!(h.node("dash").rec >= 3, "REC += 1 per error seen");

        track.extend(drive_story(&mut h, 400 * MS));
        // It went error-passive, then bus-off, then recovered (ABOM).
        let states: Vec<CanErrorState> = {
            let mut v: Vec<CanErrorState> = track.iter().map(|t| t.3).collect();
            v.dedup();
            v
        };
        assert_eq!(
            &states[..4],
            &[
                CanErrorState::Active,
                CanErrorState::Passive,
                CanErrorState::BusOff,
                CanErrorState::Active
            ],
            "{states:?}"
        );
        // Bus-off recovery took 128 x 11 recessive bit times of the bus at
        // least (2.816 ms at 500 kbit/s).
        let off = track
            .iter()
            .find(|t| t.3 == CanErrorState::BusOff)
            .unwrap()
            .0;
        let back = track
            .iter()
            .find(|t| t.0 > off && t.3 == CanErrorState::Active)
            .unwrap()
            .0;
        let recovery = 128 * 11 * BIT_500K;
        // Track times are round ends; allow one round either way.
        assert!(
            back - off + h.bus.lookahead_ps() >= recovery,
            "{} ps",
            back - off
        );
        // An engine frame on the bus inside the window breaks the idle run
        // (it counts once, for its 11-bit recessive tail), so allow a few.
        assert!(back - off < recovery + MS / 2, "{} ps", back - off);
        // Once passive the sensor no longer destroyed the engine's frames:
        // the dashboard got engine frames, and the engine stayed
        // error-active up to the sensor's first bus-off. (After that the
        // sensor, with ABOM and a frame it can never send, rejoins
        // error-active every few milliseconds and destroys an engine frame
        // now and then — a babbling node, as on a real bus.)
        assert!(!h.node("dash").rx.is_empty());
        assert!(track
            .iter()
            .take_while(|t| t.3 != CanErrorState::BusOff)
            .all(|t| t.5 == CanErrorState::Active));
        // The report shows it.
        let rep = h.bus.report(h.now);
        assert!(rep.totals.ok > 0 && rep.totals.error > 0);
        let sensor = rep.nodes.iter().find(|n| n.node == "sensor").unwrap();
        assert_eq!(sensor.bitrate, Some(250_000));
    }

    #[test]
    fn an_error_passive_mismatched_node_no_longer_destroys_frames() {
        let mut h = story(&["dash", "engine", "sensor"]);
        // Put the sensor into error-passive by its own failed transmissions
        // (16 x TEC += 8), then let the engine send while it keeps retrying.
        h.node("sensor").send(std(0x200, &[1]), 0);
        loop {
            h.round();
            if h.node("sensor").state == CanErrorState::Passive {
                break;
            }
        }
        let before = h.node("dash").rx.len();
        let c = ps_to_cycles(h.now);
        h.node("engine").send(std(0x100, &[7]), c);
        h.run_until(h.now + MS);
        assert!(h.node("dash").rx.len() > before, "the engine frame arrived");
        let arrived = h.node("dash").rx[0].0;
        assert!(h
            .node("engine")
            .outcomes
            .contains(&(arrived, CanTxOutcome::Ok)));
    }

    #[test]
    fn results_do_not_depend_on_manifest_order() {
        let mut a = story(&["dash", "engine", "sensor"]);
        let mut b = story(&["sensor", "engine", "dash"]);
        let ta = drive_story(&mut a, 120 * MS);
        let tb = drive_story(&mut b, 120 * MS);
        assert_eq!(ta, tb);
        assert_eq!(a.bus.report(a.now), b.bus.report(b.now));
    }

    #[test]
    fn the_frame_log_is_bounded_and_counts_what_it_dropped() {
        let mut quiet = FakeNode::new(BIT_500K);
        quiet.on_bus = false;
        let mut h = Harness::new(vec![("lone", FakeNode::new(BIT_500K)), ("quiet", quiet)]);
        h.node("lone").send(std(0x1, &[]), 0);
        while h.bus.report(h.now).totals.no_ack < FRAME_LOG_CAP as u64 + 10 {
            h.round();
        }
        let rep = h.bus.report(h.now);
        assert_eq!(rep.frames.len(), FRAME_LOG_CAP);
        assert_eq!(rep.frames_dropped, rep.totals.no_ack - FRAME_LOG_CAP as u64);
        assert!(rep.frames.windows(2).all(|w| w[0].seq + 1 == w[1].seq));
    }
}
