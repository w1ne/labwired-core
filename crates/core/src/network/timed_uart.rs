// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! A timed UART network: serial links between world nodes that take the time
//! the wire takes.
//!
//! The older cross-link medium ([`super::VirtualWireBus`]) hands a byte to the
//! peer the instant firmware writes it, into a queue with no bound. That is
//! enough to prove two firmwares speak the same protocol, and useless for the
//! question a network team asks: *how long does a message take across N hops,
//! and what happens to the buffers when a link slows down or a node restarts?*
//!
//! Here every character is a waveform with a start time:
//!
//! * The sender's USART shifts it out at the baud rate its `BRR` programs, in
//!   the frame format its `CR1`/`CR2` program (data bits, parity, stop bits).
//!   Characters from one sender never overlap: the next start bit follows the
//!   last stop bit.
//! * The link adds a configurable propagation delay and an optional jitter,
//!   drawn from a per-link generator seeded from the manifest, so a run is
//!   reproducible bit for bit.
//! * The receiving USART samples the waveform at ITS OWN baud rate and format,
//!   in the middle of each of its bit cells. Two ends that disagree on the
//!   baud rate therefore read garbage and see framing errors (FE) — the
//!   character is not delivered by magic.
//! * The receiver holds one character (the data register). A character that
//!   completes while the previous one is still unread is lost and sets the
//!   overrun flag (ORE). There is no host-side receive queue.
//!
//! What lives here is only the medium: characters on the wire, per-link
//! statistics and one timeline. The USART register behaviour (TXE/TC/RXNE/ORE,
//! interrupts) is in `peripherals::uart`, in its timed mode.
//!
//! Time is kept in picoseconds of world time: a 115200-baud bit is
//! 8 680 555 ps, and at 84 MHz one CPU cycle is 11 904 ps, so integer ps keep
//! both exact enough that golden numbers can be compared to the cycle.

use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Picoseconds per second.
pub const PS_PER_SEC: u128 = 1_000_000_000_000;

/// The timeline keeps at most this many events; later ones are counted in
/// [`NetReport::events_dropped`] instead of growing memory without bound.
pub const TIMELINE_CAP: usize = 250_000;

/// At most this many tagged messages are tracked (oldest ids are dropped).
pub const MESSAGE_CAP: usize = 65_536;

/// Convert engine cycles at `hz` to picoseconds.
pub fn cycles_to_ps(cycles: u64, hz: u64) -> u64 {
    if hz == 0 {
        return 0;
    }
    (u128::from(cycles) * PS_PER_SEC / u128::from(hz)) as u64
}

/// The first cycle at `hz` at or after `ps`.
pub fn ps_to_cycles_ceil(ps: u64, hz: u64) -> u64 {
    (u128::from(ps) * u128::from(hz)).div_ceil(PS_PER_SEC) as u64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Parity {
    #[default]
    None,
    Even,
    Odd,
}

/// A USART's current line configuration, as its registers program it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct LineFormat {
    /// One bit period, ps. 0 = no baud rate programmed.
    pub bit_ps: u64,
    /// Data bits, parity excluded (7, 8 or 9).
    pub data_bits: u8,
    pub parity: Parity,
    /// Stop bits in half-bit units: 1 = 0.5, 2 = 1, 3 = 1.5, 4 = 2.
    pub stop_half_bits: u8,
    /// UE and RE: the receiver is sampling the line.
    pub rx_enabled: bool,
    /// UE and TE: the transmitter drives the line.
    pub tx_enabled: bool,
}

impl LineFormat {
    pub fn configured(&self) -> bool {
        self.bit_ps > 0 && self.data_bits > 0
    }
    /// Start + data + parity: the bits sampled before the first stop bit.
    pub fn bits_before_stop(&self) -> u64 {
        1 + u64::from(self.data_bits) + u64::from(self.parity != Parity::None)
    }
    /// Whole character on the wire, stop bits included, ps.
    pub fn frame_ps(&self) -> u64 {
        self.bit_ps * self.bits_before_stop() + self.bit_ps * u64::from(self.stop_half_bits) / 2
    }
    /// Offset from the start-bit edge to the middle of the first stop bit —
    /// where the receiver decides the character (and sets RXNE), ps.
    pub fn sample_offset_ps(&self) -> u64 {
        self.bit_ps * self.bits_before_stop() + self.bit_ps / 2
    }
    pub fn baud(&self) -> f64 {
        if self.bit_ps == 0 {
            0.0
        } else {
            PS_PER_SEC as f64 / self.bit_ps as f64
        }
    }
    fn parity_bit(&self, value: u16) -> bool {
        let mask = (1u32 << self.data_bits) - 1;
        let ones = (u32::from(value) & mask).count_ones() & 1 == 1;
        match self.parity {
            Parity::None => false,
            Parity::Even => ones,
            Parity::Odd => !ones,
        }
    }
    /// Line level of bit `idx` (0 = start) of `value`; stop and idle are 1.
    pub(crate) fn level_of_bit(&self, value: u16, idx: u64) -> bool {
        let data = u64::from(self.data_bits);
        if idx == 0 {
            false
        } else if idx <= data {
            (value >> (idx - 1)) & 1 == 1
        } else if idx == data + 1 && self.parity != Parity::None {
            self.parity_bit(value)
        } else {
            true
        }
    }
}

/// One character on the wire, heading to a receiving port.
#[derive(Debug, Clone)]
struct Wire {
    /// Global character number, for the timeline.
    id: u64,
    value: u16,
    /// The sender's format when the character started.
    tx: LineFormat,
    tx_start_ps: u64,
    /// Start-bit edge as the receiver sees it (after delay and jitter).
    t0_ps: u64,
    /// From this receiver time on, the line reads idle (1): the link was cut
    /// or the sender was reset mid-character.
    truncate_ps: Option<u64>,
    link: usize,
    dir: usize,
    /// Negative-control mode: the character is due the instant it starts.
    instant: bool,
    /// A receiver character started on one of this character's edges.
    decoded: bool,
}

impl Wire {
    /// The receiver time the line stops carrying this character.
    fn end_ps(&self) -> u64 {
        let end = self.t0_ps + self.tx.frame_ps();
        self.truncate_ps.map_or(end, |c| c.min(end))
    }

    fn level_at(&self, t: u64) -> bool {
        if t < self.t0_ps {
            return true;
        }
        if self.truncate_ps.is_some_and(|cut| t >= cut) {
            return true;
        }
        if self.tx.bit_ps == 0 {
            return true;
        }
        let idx = (t - self.t0_ps) / self.tx.bit_ps;
        self.tx.level_of_bit(self.value, idx)
    }
}

/// The line a receiver sees: the characters on the wire toward it, in time
/// order and never overlapping, idle (1) between and after them.
fn line_level(inbox: &VecDeque<Wire>, t: u64) -> bool {
    for w in inbox {
        if t < w.t0_ps {
            return true;
        }
        if t < w.end_ps() {
            return w.level_at(t);
        }
    }
    true
}

/// The first falling edge of the line at or after `from`, and the character
/// it belongs to. Every start bit is one; with a mismatched receiver a data
/// bit can be one too.
fn next_falling_edge(inbox: &VecDeque<Wire>, from: u64) -> Option<(u64, usize)> {
    for (i, w) in inbox.iter().enumerate() {
        if w.end_ps() <= from || w.tx.bit_ps == 0 || w.instant {
            continue;
        }
        let mut prev = true; // the line before a start bit is idle or a stop bit
        for k in 0..=w.tx.bits_before_stop() {
            let level = w.tx.level_of_bit(w.value, k);
            let tb = w.t0_ps + k * w.tx.bit_ps;
            if tb >= w.end_ps() {
                break;
            }
            if prev && !level && tb >= from {
                return Some((tb, i));
            }
            prev = level;
        }
    }
    None
}

/// Two formats a receiver decodes correctly and within the character: same
/// framing and bit times within 1/32 (about 3 %, inside the USART's tolerance).
fn formats_match(tx: &LineFormat, rx: &LineFormat) -> bool {
    tx.data_bits == rx.data_bits
        && tx.parity == rx.parity
        && tx.bit_ps.abs_diff(rx.bit_ps) <= tx.bit_ps / 32
}

/// What a receiver made of one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    /// The data register value: data bits, plus the received parity bit in the
    /// next position up when parity is enabled (STM32 RM0368 §19.3.7).
    pub value: u16,
    pub framing_error: bool,
    pub parity_error: bool,
}

/// Sample a waveform the way a receiver in format `rx` does: a start bit at
/// mid-cell, each data (and parity) bit at mid-cell, then the first stop bit.
/// `None` when the start bit does not read 0 (a false start: nothing received).
fn decode(wire: &Wire, rx: &LineFormat) -> Option<Decoded> {
    decode_with(wire.t0_ps, rx, |t| wire.level_at(t))
}

/// [`decode`] on the composite line, for a character starting at `edge`.
fn decode_line(inbox: &VecDeque<Wire>, edge: u64, rx: &LineFormat) -> Option<Decoded> {
    decode_with(edge, rx, |t| line_level(inbox, t))
}

fn decode_with(edge: u64, rx: &LineFormat, level: impl Fn(u64) -> bool) -> Option<Decoded> {
    let bit = rx.bit_ps;
    let half = bit / 2;
    let at = |k: u64| edge + k * bit + half;
    if level(at(0)) {
        return None;
    }
    let data = u64::from(rx.data_bits);
    let mut value: u16 = 0;
    for i in 0..data {
        if level(at(1 + i)) {
            value |= 1 << i;
        }
    }
    let mut parity_error = false;
    if rx.parity != Parity::None {
        let p = level(at(1 + data));
        parity_error = p != rx.parity_bit(value);
        if p {
            value |= 1 << data;
        }
    }
    let framing_error = !level(at(rx.bits_before_stop()));
    Some(Decoded {
        value,
        framing_error,
        parity_error,
    })
}

/// A small deterministic generator for link jitter (xorshift64*).
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64, stream: u64) -> Self {
        // SplitMix64 of (seed, stream) so neighbouring links are uncorrelated
        // and a zero seed still yields a nonzero state.
        let mut z = seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Self(z | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `[0, max]`.
    fn up_to(&mut self, max: u64) -> u64 {
        if max == 0 {
            0
        } else {
            self.next() % (max + 1)
        }
    }
}

/// What kind of thing happened, on the one timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetEventKind {
    /// A character's start bit left the sender.
    TxStart,
    /// The receiver sampled the character's stop bit and loaded its data
    /// register (RXNE set).
    Deliver,
    /// The receiver's interrupt line rose because of received data.
    RxIrq,
    /// A character completed while the data register was still full: lost.
    Overrun,
    /// Received with a stop bit that read 0.
    FramingError,
    /// Received with a parity bit that disagrees with the data.
    ParityError,
    /// A character that no USART received (link cut, receiver disabled or
    /// reset, false start). `detail` says which.
    Drop,
    NodeReset,
    LinkDelay,
    LinkCut,
    LinkRestore,
    /// An application-level marker (a watched GPIO edge).
    Marker,
    /// A tagged message completed at a receiver.
    MessageDelivered,
}

/// One entry of the unified network timeline.
#[derive(Debug, Clone, Serialize)]
pub struct NetEvent {
    pub seq: u64,
    /// World time, ps.
    pub t_ps: u64,
    pub node: String,
    pub kind: NetEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uart: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u32>,
    /// Character id (TxStart/Deliver/Overrun) — joins a delivery to its
    /// transmission: `(link * 2 + direction) << 40 | n`, n counting from 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub char_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Per-direction link statistics.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DirStats {
    pub from_node: String,
    pub from_uart: String,
    pub to_node: String,
    pub to_uart: String,
    pub chars_sent: u64,
    pub chars_delivered: u64,
    pub framing_errors: u64,
    pub parity_errors: u64,
    pub overruns: u64,
    pub dropped_link_cut: u64,
    pub dropped_rx_disabled: u64,
    /// Start edges whose start bit did not read 0 at mid-bit (noise).
    pub false_starts: u64,
    /// Characters on which no receiver character started: a mismatched
    /// receiver sampled across them.
    pub misframed: u64,
    /// Characters cut short (link cut or sender reset mid-character).
    pub truncated: u64,
    /// Characters the receiver acted on after its own clock had already
    /// passed their time — a synchronisation fault; 0 in a correct run.
    pub late: u64,
    /// Most characters on the wire at once (propagation delay makes this >1).
    pub max_in_flight: u64,
    /// Transmit start → receiver stop-bit sample, delivered characters, ps.
    pub latency_min_ps: Option<u64>,
    pub latency_max_ps: Option<u64>,
    latency_sum_ps: u128,
    pub first_tx_ps: Option<u64>,
    pub last_delivery_ps: Option<u64>,
    /// Sum of character times on the wire, ps (line busy time).
    pub busy_ps: u64,
}

impl DirStats {
    pub fn latency_mean_ps(&self) -> Option<f64> {
        (self.chars_delivered > 0).then(|| self.latency_sum_ps as f64 / self.chars_delivered as f64)
    }
}

#[derive(Debug)]
struct Port {
    node: String,
    uart: String,
    link: usize,
    side: usize,
    fmt: LineFormat,
    /// Characters on the wire toward this port, oldest first. This is the
    /// WIRE, not a buffer: its length is bounded by delay / character time.
    inbox: VecDeque<Wire>,
    /// Receiver time this port has been serviced up to, ps.
    serviced_ps: u64,
    /// The receiver looks for its next start edge from here, ps.
    hunt_ps: u64,
    rx_sniff: Sniffer,
    tx_sniff: Sniffer,
    /// Digital transceiver propagation at this endpoint; zero on bare UARTs.
    tx_path_delay_ps: u64,
    rx_path_delay_ps: u64,
}

#[derive(Debug)]
struct Link {
    ports: [usize; 2],
    delay_ps: u64,
    jitter_ps: u64,
    connected: bool,
    rng: Rng,
    /// Receiver-side time the line of each direction is free again.
    wire_free_ps: [u64; 2],
    stats: [DirStats; 2],
}

/// Message tagging, mirrored from the manifest.
#[derive(Debug, Clone)]
pub struct MessageTagging {
    pub sync: u8,
    pub length: usize,
    pub id_offset: usize,
    pub id_bytes: usize,
    pub hop_offset: Option<usize>,
    pub checksum_xor: bool,
}

#[derive(Debug, Default)]
struct Sniffer {
    buf: Vec<u8>,
    first_ps: u64,
}

struct Parsed {
    id: u32,
    hop: Option<u8>,
    first_ps: u64,
}

impl Sniffer {
    fn reset(&mut self) {
        self.buf.clear();
    }
    fn feed(&mut self, tag: &MessageTagging, byte: u8, t: u64) -> Option<Parsed> {
        if self.buf.is_empty() {
            if byte != tag.sync {
                return None;
            }
            self.first_ps = t;
        }
        self.buf.push(byte);
        if self.buf.len() < tag.length {
            return None;
        }
        let buf = std::mem::take(&mut self.buf);
        if tag.checksum_xor {
            let x = buf[1..tag.length - 1].iter().fold(0u8, |a, b| a ^ b);
            if x != buf[tag.length - 1] {
                return None;
            }
        }
        let mut id = 0u32;
        for i in 0..tag.id_bytes {
            id |= u32::from(buf[tag.id_offset + i]) << (8 * i);
        }
        Some(Parsed {
            id,
            hop: tag.hop_offset.map(|h| buf[h]),
            first_ps: self.first_ps,
        })
    }
}

/// One tagged message's journey.
#[derive(Debug, Clone, Serialize)]
pub struct MessageRecord {
    pub id: u32,
    pub origin_node: Option<String>,
    /// Start bit of the message's first character at its origin, ps.
    pub origin_ps: Option<u64>,
    pub deliveries: Vec<MessageDelivery>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageDelivery {
    pub node: String,
    /// Stop-bit sample of the last character, ps.
    pub t_ps: u64,
    /// `t_ps - origin_ps`, when the origin was seen.
    pub latency_ps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hop: Option<u8>,
}

#[derive(Debug, Default)]
struct Inner {
    ports: Vec<Port>,
    links: Vec<Link>,
    events: Vec<NetEvent>,
    events_dropped: u64,
    next_seq: u64,
    tagging: Option<MessageTagging>,
    messages: BTreeMap<u32, MessageRecord>,
    node_resets: BTreeMap<String, u64>,
    instant: bool,
}

impl Inner {
    fn record(&mut self, mut ev: NetEvent) {
        ev.seq = self.next_seq;
        self.next_seq += 1;
        if self.events.len() >= TIMELINE_CAP {
            self.events_dropped += 1;
            return;
        }
        self.events.push(ev);
    }

    fn event(&mut self, t_ps: u64, node: &str, kind: NetEventKind) -> NetEvent {
        NetEvent {
            seq: 0,
            t_ps,
            node: node.to_string(),
            kind,
            link: None,
            uart: None,
            value: None,
            char_id: None,
            msg: None,
            detail: None,
        }
    }

    fn port_event(&self, port: usize, t_ps: u64, kind: NetEventKind) -> NetEvent {
        let p = &self.ports[port];
        NetEvent {
            seq: 0,
            t_ps,
            node: p.node.clone(),
            kind,
            link: Some(p.link as u32),
            uart: Some(p.uart.clone()),
            value: None,
            char_id: None,
            msg: None,
            detail: None,
        }
    }

    fn message_mut(&mut self, id: u32) -> &mut MessageRecord {
        if !self.messages.contains_key(&id) && self.messages.len() >= MESSAGE_CAP {
            let oldest = *self.messages.keys().next().expect("non-empty");
            self.messages.remove(&oldest);
        }
        self.messages.entry(id).or_insert_with(|| MessageRecord {
            id,
            origin_node: None,
            origin_ps: None,
            deliveries: Vec::new(),
        })
    }
}

/// The shared medium of one world's timed UART network. Clones share it.
#[derive(Clone, Default)]
pub struct TimedUartNet {
    inner: Arc<Mutex<Inner>>,
}

/// One end of a timed link, attached to one USART.
#[derive(Clone)]
pub struct TimedUartPort {
    net: Arc<Mutex<Inner>>,
    port: usize,
    hz: u64,
}

impl std::fmt::Debug for TimedUartPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimedUartPort")
            .field("port", &self.port)
            .field("hz", &self.hz)
            .finish()
    }
}

/// Why a character reached no data register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropReason {
    RxDisabled,
    FalseStart,
}

impl TimedUartNet {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_message_tagging(&self, tagging: Option<MessageTagging>) {
        self.lock().tagging = tagging;
    }

    /// Add a link between `(node_a, uart_a)` and `(node_b, uart_b)`; returns
    /// its index and the two ports to attach to the USARTs.
    #[allow(clippy::too_many_arguments)]
    pub fn add_link(
        &self,
        node_a: (&str, &str, u64),
        node_b: (&str, &str, u64),
        delay_ps: u64,
        jitter_ps: u64,
        seed: u64,
    ) -> (usize, TimedUartPort, TimedUartPort) {
        let mut g = self.lock();
        let link = g.links.len();
        let pa = g.ports.len();
        let pb = pa + 1;
        for (side, (node, uart, _hz)) in [node_a, node_b].into_iter().enumerate() {
            g.ports.push(Port {
                node: node.to_string(),
                uart: uart.to_string(),
                link,
                side,
                fmt: LineFormat::default(),
                inbox: VecDeque::new(),
                serviced_ps: 0,
                hunt_ps: 0,
                rx_sniff: Sniffer::default(),
                tx_sniff: Sniffer::default(),
                tx_path_delay_ps: 0,
                rx_path_delay_ps: 0,
            });
        }
        let dir_stats = |from: &Port, to: &Port| DirStats {
            from_node: from.node.clone(),
            from_uart: from.uart.clone(),
            to_node: to.node.clone(),
            to_uart: to.uart.clone(),
            ..Default::default()
        };
        let stats = [
            dir_stats(&g.ports[pa], &g.ports[pb]),
            dir_stats(&g.ports[pb], &g.ports[pa]),
        ];
        g.links.push(Link {
            ports: [pa, pb],
            delay_ps,
            jitter_ps,
            connected: true,
            rng: Rng::new(seed, link as u64),
            wire_free_ps: [0, 0],
            stats,
        });
        drop(g);
        (
            link,
            TimedUartPort {
                net: self.inner.clone(),
                port: pa,
                hz: node_a.2,
            },
            TimedUartPort {
                net: self.inner.clone(),
                port: pb,
                hz: node_b.2,
            },
        )
    }

    /// NEGATIVE CONTROL ONLY: deliver every character the instant its start
    /// bit leaves — the untimed behaviour this module replaces. Exists so a
    /// test can show its latency assertions fail without the wire model.
    #[doc(hidden)]
    pub fn debug_instant_delivery(&self, on: bool) {
        self.lock().instant = on;
    }

    pub fn link_count(&self) -> usize {
        self.lock().links.len()
    }

    /// The shortest time any character can take from a sender's start bit to
    /// the moment a receiver acts on it, given every port's current format
    /// and every link's current delay. A synchronisation round no longer than
    /// this cannot deliver a character into a receiver's past. `None` when no
    /// port can transmit yet.
    pub fn lookahead_ps(&self) -> Option<u64> {
        let g = self.lock();
        if g.instant {
            return None;
        }
        let mut best: Option<u64> = None;
        for link in &g.links {
            for dir in 0..2 {
                let tx = &g.ports[link.ports[dir]].fmt;
                let rx = &g.ports[link.ports[dir ^ 1]].fmt;
                if !tx.configured() {
                    continue;
                }
                // Every format samples at least 9 bits before the stop bit
                // (start + 7 data + parity, or start + 8 data), so the earliest
                // any receiver acts is 9.5 bit times after the edge; an
                // unconfigured receiver drops at the end of the sender's frame.
                let mut bit = tx.bit_ps;
                if rx.configured() {
                    bit = bit.min(rx.bit_ps);
                }
                // A mismatched receiver samples past the character it started
                // on, into line time the sender may not have decided yet, so
                // the round shrinks to one 16x oversampling tick past the
                // delay. Its result is exact to that tick, not to the cycle.
                let la = if rx.configured() && !formats_match(tx, rx) {
                    link.delay_ps + bit / 16
                } else {
                    link.delay_ps + bit * 19 / 2
                };
                best = Some(best.map_or(la, |b| b.min(la)));
            }
        }
        best
    }

    /// Change a link's propagation delay (and jitter), at world time `t_ps`.
    pub fn set_link_delay(&self, link: usize, delay_ps: u64, jitter_ps: Option<u64>, t_ps: u64) {
        let mut g = self.lock();
        let Some(l) = g.links.get_mut(link) else {
            return;
        };
        l.delay_ps = delay_ps;
        if let Some(j) = jitter_ps {
            l.jitter_ps = j;
        }
        let jitter = l.jitter_ps;
        let a = g.ports[g.links[link].ports[0]].node.clone();
        let mut ev = g.event(t_ps, &a, NetEventKind::LinkDelay);
        ev.link = Some(link as u32);
        ev.detail = Some(format!("delay_ps={delay_ps} jitter_ps={jitter}"));
        g.record(ev);
    }

    /// Cut (`connected = false`) or restore a link at world time `t_ps`. A
    /// character on the wire at the cut is truncated there: the receiver sees
    /// the idle line from then on.
    pub fn set_link_connected(&self, link: usize, connected: bool, t_ps: u64) {
        let mut g = self.lock();
        if link >= g.links.len() {
            return;
        }
        g.links[link].connected = connected;
        if !connected {
            let ports = g.links[link].ports;
            for (dir, &rx_port) in ports.iter().rev().enumerate() {
                // `ports.iter().rev()` visits side 1 then side 0: the inbox of
                // side 1 carries direction 0 (0 → 1), and vice versa.
                let mut removed = 0u64;
                let mut truncated = 0u64;
                let inbox = &mut g.ports[rx_port].inbox;
                inbox.retain_mut(|w| {
                    if w.t0_ps >= t_ps {
                        removed += 1;
                        false
                    } else {
                        if w.t0_ps + w.tx.frame_ps() > t_ps {
                            w.truncate_ps = Some(w.truncate_ps.map_or(t_ps, |c| c.min(t_ps)));
                            truncated += 1;
                        }
                        true
                    }
                });
                let s = &mut g.links[link].stats[dir];
                s.dropped_link_cut += removed;
                s.truncated += truncated;
            }
        }
        let a = g.ports[g.links[link].ports[0]].node.clone();
        let kind = if connected {
            NetEventKind::LinkRestore
        } else {
            NetEventKind::LinkCut
        };
        let mut ev = g.event(t_ps, &a, kind);
        ev.link = Some(link as u32);
        g.record(ev);
    }

    /// Put a node reset on the timeline.
    pub fn record_node_reset(&self, node: &str, t_ps: u64) {
        let mut g = self.lock();
        *g.node_resets.entry(node.to_string()).or_default() += 1;
        let ev = g.event(t_ps, node, NetEventKind::NodeReset);
        g.record(ev);
    }

    /// Put an application marker (a GPIO edge) on the timeline.
    pub fn record_marker(&self, node: &str, name: &str, level: bool, t_ps: u64) {
        let mut g = self.lock();
        let mut ev = g.event(t_ps, node, NetEventKind::Marker);
        ev.value = Some(u32::from(level));
        ev.detail = Some(name.to_string());
        g.record(ev);
    }

    /// The whole network state: links with statistics, tagged messages, and
    /// the timeline from sequence number `since` on, ordered by time.
    pub fn report(&self, since: u64, now_ps: u64) -> NetReport {
        let g = self.lock();
        let links = g
            .links
            .iter()
            .enumerate()
            .map(|(i, l)| LinkReport {
                id: i as u32,
                a_node: g.ports[l.ports[0]].node.clone(),
                a_uart: g.ports[l.ports[0]].uart.clone(),
                b_node: g.ports[l.ports[1]].node.clone(),
                b_uart: g.ports[l.ports[1]].uart.clone(),
                a_format: g.ports[l.ports[0]].fmt,
                b_format: g.ports[l.ports[1]].fmt,
                delay_ps: l.delay_ps,
                jitter_ps: l.jitter_ps,
                connected: l.connected,
                directions: l
                    .stats
                    .iter()
                    .map(|s| DirReport::from_stats(s, now_ps))
                    .collect(),
            })
            .collect();
        let mut events: Vec<NetEvent> = g
            .events
            .iter()
            .filter(|e| e.seq >= since)
            .cloned()
            .collect();
        events.sort_by(|a, b| (a.t_ps, &a.node, a.seq).cmp(&(b.t_ps, &b.node, b.seq)));
        NetReport {
            now_ps,
            links,
            next_cursor: g.next_seq,
            events_dropped: g.events_dropped,
            events,
            messages: g
                .messages
                .values()
                .map(|m| {
                    let mut m = m.clone();
                    m.deliveries
                        .sort_by(|a, b| (a.t_ps, &a.node).cmp(&(b.t_ps, &b.node)));
                    m
                })
                .collect(),
            node_resets: g.node_resets.clone(),
        }
    }
}

impl TimedUartPort {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.net.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn hz(&self) -> u64 {
        self.hz
    }

    /// Add endpoint transceiver propagation separately from cable/link delay.
    /// TX start events stay on the MCU UART pin; receive sampling includes the
    /// sending driver's and receiving transceiver's delays.
    pub fn set_path_delays(&self, tx_ps: u64, rx_ps: u64) {
        let mut g = self.lock();
        g.ports[self.port].tx_path_delay_ps = tx_ps;
        g.ports[self.port].rx_path_delay_ps = rx_ps;
    }

    /// The USART's registers changed: record its line format.
    pub fn publish_format(&self, fmt: LineFormat) {
        self.lock().ports[self.port].fmt = fmt;
    }

    /// A character's start bit leaves this port at engine cycle `start_cycle`.
    pub fn transmit(&self, start_cycle: u64, value: u16, fmt: LineFormat) {
        let tx_start_ps = cycles_to_ps(start_cycle, self.hz);
        let mut g = self.lock();
        let inner = &mut *g;
        let port = &inner.ports[self.port];
        let (link_idx, dir) = (port.link, port.side);
        // Numbered per link direction, so the id does not depend on the order
        // nodes run in: `(link * 2 + dir) << 40 | n`.
        let sent = inner.links[link_idx].stats[dir].chars_sent;
        let char_id = (((link_idx * 2 + dir) as u64) << 40) | sent;

        let mut ev = inner.port_event(self.port, tx_start_ps, NetEventKind::TxStart);
        ev.value = Some(u32::from(value));
        ev.char_id = Some(char_id);
        inner.record(ev);

        // Message tagging on the sending side: the first sender of an id is
        // its origin.
        if let Some(tag) = inner.tagging.clone() {
            let parsed =
                inner.ports[self.port]
                    .tx_sniff
                    .feed(&tag, (value & 0xFF) as u8, tx_start_ps);
            if let Some(p) = parsed {
                let node = inner.ports[self.port].node.clone();
                let rec = inner.message_mut(p.id);
                if rec.origin_ps.is_none() {
                    rec.origin_ps = Some(p.first_ps);
                    rec.origin_node = Some(node);
                }
            }
        }

        let rx_port = inner.links[link_idx].ports[dir ^ 1];
        let path_delay = inner.ports[self.port]
            .tx_path_delay_ps
            .saturating_add(inner.ports[rx_port].rx_path_delay_ps);
        let link = &mut inner.links[link_idx];
        let stats = &mut link.stats[dir];
        stats.chars_sent += 1;
        stats.first_tx_ps.get_or_insert(tx_start_ps);
        stats.busy_ps += fmt.frame_ps();
        if !link.connected {
            stats.dropped_link_cut += 1;
            let mut ev = inner.port_event(self.port, tx_start_ps, NetEventKind::Drop);
            ev.char_id = Some(char_id);
            ev.value = Some(u32::from(value));
            ev.detail = Some("link_cut".into());
            inner.record(ev);
            return;
        }
        let instant = inner.instant;
        let t0 = if instant {
            tx_start_ps
        } else {
            let jitter = link.rng.up_to(link.jitter_ps);
            // A wire carries one character at a time: a jittered start may not
            // overtake the previous character's end.
            let t0 = tx_start_ps
                .saturating_add(link.delay_ps)
                .saturating_add(jitter)
                .saturating_add(path_delay)
                .max(link.wire_free_ps[dir]);
            link.wire_free_ps[dir] = t0 + fmt.frame_ps();
            t0
        };
        let rx_port = link.ports[dir ^ 1];
        let inbox = &mut inner.ports[rx_port].inbox;
        inbox.push_back(Wire {
            id: char_id,
            value,
            tx: fmt,
            tx_start_ps,
            t0_ps: t0,
            truncate_ps: None,
            link: link_idx,
            dir,
            instant,
            decoded: false,
        });
        let depth = inbox.len() as u64;
        let stats = &mut inner.links[link_idx].stats[dir];
        stats.max_in_flight = stats.max_in_flight.max(depth);
    }

    /// The sender was reset at `cycle`: a character it was shifting out stops
    /// there, and the receiver sees the idle line for the rest of it.
    pub fn abort_tx(&self, cycle: u64) {
        let t = cycles_to_ps(cycle, self.hz);
        let mut g = self.lock();
        let (link_idx, dir) = (g.ports[self.port].link, g.ports[self.port].side);
        let rx_port = g.links[link_idx].ports[dir ^ 1];
        let mut truncated = 0;
        for w in g.ports[rx_port].inbox.iter_mut() {
            if w.tx_start_ps < t && w.tx_start_ps + w.tx.frame_ps() > t {
                let offset = w.t0_ps - w.tx_start_ps;
                w.truncate_ps = Some(t + offset);
                truncated += 1;
            }
        }
        let link = &mut g.links[link_idx];
        link.stats[dir].truncated += truncated;
        link.wire_free_ps[dir] = link.wire_free_ps[dir].min(t + link.delay_ps);
        g.ports[self.port].tx_sniff.reset();
        g.ports[self.port].rx_sniff.reset();
    }

    /// When the receiver (in format `rx`) must next act, ps.
    fn next_due_ps(port: &Port, rx: &LineFormat) -> Option<u64> {
        let front = port
            .inbox
            .iter()
            .find(|w| w.instant || w.end_ps() > port.hunt_ps)?;
        if front.instant {
            return Some(front.t0_ps);
        }
        if !(rx.configured() && rx.rx_enabled) {
            // A disabled receiver drops each character it has not already
            // taken, at the character's end.
            return port
                .inbox
                .iter()
                .find(|w| !w.decoded && w.end_ps() > port.hunt_ps)
                .map(Wire::end_ps);
        }
        let (edge, _) = next_falling_edge(&port.inbox, port.hunt_ps)?;
        Some(edge + rx.sample_offset_ps())
    }

    /// The first engine cycle at which a character on the wire toward this
    /// port needs the receiver (in format `rx`) to act.
    pub fn next_due_cycle(&self, rx: &LineFormat) -> Option<u64> {
        let g = self.lock();
        Self::next_due_ps(&g.ports[self.port], rx).map(|ps| ps_to_cycles_ceil(ps, self.hz))
    }

    /// Act on every character due by engine cycle `now_cycle`, in wire order.
    /// `rdr_full` is the receiver's RXNE: a decoded character while it is set
    /// is an overrun and is lost. Returns the character loaded into the data
    /// register (at most one), and the number of overruns.
    pub fn receive_due(
        &self,
        now_cycle: u64,
        rx: &LineFormat,
        rdr_full: bool,
    ) -> (Option<Decoded>, u32) {
        let (loaded, overruns, _) = self.receive_due_policy(now_cycle, rx, rdr_full, false, false);
        (loaded, overruns)
    }

    /// USART policy applied per decoded character, including late service of
    /// multiple characters. Error bits are PE=1 and FE=2. Kept separate from
    /// the legacy entry point so other UART clients retain their behavior.
    pub(crate) fn receive_due_policy(
        &self,
        now_cycle: u64,
        rx: &LineFormat,
        rdr_full: bool,
        overwrite: bool,
        drop_errors: bool,
    ) -> (Option<Decoded>, u32, u8) {
        let now = cycles_to_ps(now_cycle, self.hz);
        let mut g = self.lock();
        let inner = &mut *g;
        let mut full = rdr_full;
        let mut loaded = None;
        let mut overruns = 0;
        let mut error_flags = 0;
        let enabled = rx.configured() && rx.rx_enabled;
        loop {
            // Retire characters the receiver has passed without starting on.
            loop {
                let port = &mut inner.ports[self.port];
                let Some(front) = port.inbox.front() else {
                    break;
                };
                if front.instant || front.end_ps() > port.hunt_ps {
                    break;
                }
                let w = port.inbox.pop_front().expect("front exists");
                if !w.decoded {
                    inner.links[w.link].stats[w.dir].misframed += 1;
                    let mut ev = inner.port_event(self.port, w.end_ps(), NetEventKind::Drop);
                    ev.char_id = Some(w.id);
                    ev.detail = Some("misframed".into());
                    inner.record(ev);
                }
            }
            let port = &mut inner.ports[self.port];
            let Some(due) = Self::next_due_ps(port, rx) else {
                break;
            };
            if due > now {
                break;
            }
            if due < port.serviced_ps {
                // Only reachable if a round outran the lookahead.
                if let Some(w) = port.inbox.front() {
                    inner.links[w.link].stats[w.dir].late += 1;
                }
            }
            let port = &mut inner.ports[self.port];
            let instant_front = port.inbox.front().is_some_and(|w| w.instant);
            // (wire index the character started on, what the receiver read)
            let (idx, outcome) = if instant_front {
                let w = &port.inbox[0];
                (0, decode(w, rx).ok_or(DropReason::FalseStart))
            } else if !enabled {
                let i = port
                    .inbox
                    .iter()
                    .position(|w| !w.decoded && w.end_ps() > port.hunt_ps)
                    .expect("next_due_ps found one");
                port.hunt_ps = due;
                (i, Err(DropReason::RxDisabled))
            } else {
                let (edge, i) =
                    next_falling_edge(&port.inbox, port.hunt_ps).expect("next_due_ps found one");
                let d = decode_line(&port.inbox, edge, rx);
                // The receiver hunts for the next start edge after this
                // character's stop-bit sample (or, on a false start, after
                // the edge that fooled it).
                port.hunt_ps = if d.is_some() { due + 1 } else { edge + 1 };
                (i, d.ok_or(DropReason::FalseStart))
            };
            let port = &mut inner.ports[self.port];
            let (wid, tx_start, link_idx, dir) = {
                let w = &mut port.inbox[idx];
                if outcome.is_ok() || instant_front {
                    w.decoded = true;
                }
                (w.id, w.tx_start_ps, w.link, w.dir)
            };
            if instant_front {
                port.inbox.pop_front();
            } else if !enabled {
                port.inbox[idx].decoded = true; // accounted as dropped below
            }
            match outcome {
                Err(reason) => {
                    let stats = &mut inner.links[link_idx].stats[dir];
                    let detail = match reason {
                        DropReason::RxDisabled => {
                            stats.dropped_rx_disabled += 1;
                            "rx_disabled"
                        }
                        DropReason::FalseStart => {
                            stats.false_starts += 1;
                            "false_start"
                        }
                    };
                    inner.ports[self.port].rx_sniff.reset();
                    let mut ev = inner.port_event(self.port, due, NetEventKind::Drop);
                    ev.char_id = Some(wid);
                    ev.detail = Some(detail.into());
                    inner.record(ev);
                }
                Ok(d) => {
                    let latency = due - tx_start;
                    if full && !overwrite {
                        overruns += 1;
                        inner.links[link_idx].stats[dir].overruns += 1;
                        inner.ports[self.port].rx_sniff.reset();
                        let mut ev = inner.port_event(self.port, due, NetEventKind::Overrun);
                        ev.char_id = Some(wid);
                        ev.value = Some(u32::from(d.value));
                        inner.record(ev);
                        continue;
                    }
                    error_flags |= u8::from(d.parity_error) | (u8::from(d.framing_error) << 1);
                    if drop_errors && (d.parity_error || d.framing_error) {
                        let stats = &mut inner.links[link_idx].stats[dir];
                        stats.framing_errors += u64::from(d.framing_error);
                        stats.parity_errors += u64::from(d.parity_error);
                        inner.ports[self.port].rx_sniff.reset();
                        let mut ev = inner.port_event(
                            self.port,
                            due,
                            if d.framing_error {
                                NetEventKind::FramingError
                            } else {
                                NetEventKind::ParityError
                            },
                        );
                        ev.char_id = Some(wid);
                        ev.value = Some(u32::from(d.value));
                        ev.detail = Some("DMA DDRE=0: erroneous item discarded before RXNE".into());
                        inner.record(ev);
                        continue;
                    }
                    full = true;
                    loaded = Some(d);
                    let stats = &mut inner.links[link_idx].stats[dir];
                    stats.chars_delivered += 1;
                    stats.latency_min_ps =
                        Some(stats.latency_min_ps.map_or(latency, |m| m.min(latency)));
                    stats.latency_max_ps =
                        Some(stats.latency_max_ps.map_or(latency, |m| m.max(latency)));
                    stats.latency_sum_ps += u128::from(latency);
                    stats.last_delivery_ps = Some(due);
                    if d.framing_error {
                        stats.framing_errors += 1;
                    }
                    if d.parity_error {
                        stats.parity_errors += 1;
                    }
                    let kind = if d.framing_error {
                        NetEventKind::FramingError
                    } else if d.parity_error {
                        NetEventKind::ParityError
                    } else {
                        NetEventKind::Deliver
                    };
                    let mut ev = inner.port_event(self.port, due, kind);
                    ev.char_id = Some(wid);
                    ev.value = Some(u32::from(d.value));
                    ev.detail = Some(format!("latency_ps={latency}"));
                    inner.record(ev);
                    if d.framing_error || d.parity_error {
                        inner.ports[self.port].rx_sniff.reset();
                    } else if let Some(tag) = inner.tagging.clone() {
                        let parsed =
                            inner.ports[self.port]
                                .rx_sniff
                                .feed(&tag, (d.value & 0xFF) as u8, due);
                        if let Some(p) = parsed {
                            let node = inner.ports[self.port].node.clone();
                            let uart = inner.ports[self.port].uart.clone();
                            let rec = inner.message_mut(p.id);
                            let latency_ps = rec.origin_ps.map(|o| due.saturating_sub(o));
                            rec.deliveries.push(MessageDelivery {
                                node: node.clone(),
                                t_ps: due,
                                latency_ps,
                                hop: p.hop,
                            });
                            let _ = p.first_ps;
                            let mut ev = inner.event(due, &node, NetEventKind::MessageDelivered);
                            ev.link = Some(link_idx as u32);
                            ev.uart = Some(uart);
                            ev.msg = Some(p.id);
                            ev.detail = latency_ps.map(|l| format!("latency_ps={l}"));
                            inner.record(ev);
                        }
                    }
                }
            }
        }
        let port = &mut inner.ports[self.port];
        port.serviced_ps = port.serviced_ps.max(now);
        (loaded, overruns, error_flags)
    }

    /// The receiver's interrupt line rose because of received data (RXNE or
    /// ORE with RXNEIE).
    pub fn note_rx_irq(&self, cycle: u64, cause: &str) {
        let t = cycles_to_ps(cycle, self.hz);
        let mut g = self.lock();
        let mut ev = g.port_event(self.port, t, NetEventKind::RxIrq);
        ev.detail = Some(cause.to_string());
        g.record(ev);
    }

    /// Record a USART condition the model does not carry out (for example
    /// DMA reception), once, so a run that depends on it says so.
    pub fn note_unmodelled(&self, cycle: u64, what: &str) {
        let t = cycles_to_ps(cycle, self.hz);
        let mut g = self.lock();
        let mut ev = g.port_event(self.port, t, NetEventKind::Drop);
        ev.detail = Some(format!("unmodelled: {what}"));
        g.record(ev);
    }
}

/// One link in a [`NetReport`].
#[derive(Debug, Clone, Serialize)]
pub struct LinkReport {
    pub id: u32,
    pub a_node: String,
    pub a_uart: String,
    pub b_node: String,
    pub b_uart: String,
    pub a_format: LineFormat,
    pub b_format: LineFormat,
    pub delay_ps: u64,
    pub jitter_ps: u64,
    pub connected: bool,
    /// `[a → b, b → a]`.
    pub directions: Vec<DirReport>,
}

/// One direction of a link in a [`NetReport`].
#[derive(Debug, Clone, Serialize)]
pub struct DirReport {
    #[serde(flatten)]
    pub stats: DirStats,
    pub latency_mean_ps: Option<f64>,
    /// Delivered characters per second between the first start bit and the
    /// last delivery.
    pub throughput_chars_per_s: Option<f64>,
    /// Fraction of the run (to `now`) the line carried a character.
    pub utilization: Option<f64>,
}

impl DirReport {
    fn from_stats(s: &DirStats, now_ps: u64) -> Self {
        let throughput = match (s.first_tx_ps, s.last_delivery_ps) {
            (Some(a), Some(b)) if b > a => {
                Some(s.chars_delivered as f64 * PS_PER_SEC as f64 / (b - a) as f64)
            }
            _ => None,
        };
        let utilization = (now_ps > 0).then(|| s.busy_ps as f64 / now_ps as f64);
        Self {
            latency_mean_ps: s.latency_mean_ps(),
            stats: s.clone(),
            throughput_chars_per_s: throughput,
            utilization,
        }
    }
}

/// Everything the network knows, for a run result or the browser view.
#[derive(Debug, Clone, Serialize)]
pub struct NetReport {
    pub now_ps: u64,
    pub links: Vec<LinkReport>,
    /// Pass back as `since` to get only newer events.
    pub next_cursor: u64,
    pub events_dropped: u64,
    pub events: Vec<NetEvent>,
    pub messages: Vec<MessageRecord>,
    pub node_resets: BTreeMap<String, u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ: u64 = 84_000_000;

    fn fmt_8n1(baud: u64) -> LineFormat {
        LineFormat {
            bit_ps: (PS_PER_SEC / u128::from(baud)) as u64,
            data_bits: 8,
            parity: Parity::None,
            stop_half_bits: 2,
            rx_enabled: true,
            tx_enabled: true,
        }
    }

    fn pair(delay_ps: u64, jitter_ps: u64) -> (TimedUartNet, TimedUartPort, TimedUartPort) {
        let net = TimedUartNet::new();
        let (_, a, b) = net.add_link(
            ("a", "uart2", HZ),
            ("b", "uart1", HZ),
            delay_ps,
            jitter_ps,
            7,
        );
        (net, a, b)
    }

    #[test]
    fn a_character_arrives_one_frame_minus_half_a_stop_bit_later() {
        let (_net, a, b) = pair(0, 0);
        let f = fmt_8n1(115_200);
        a.publish_format(f);
        b.publish_format(f);
        a.transmit(0, 0x5A, f);
        let due = b.next_due_cycle(&f).unwrap();
        // 9.5 bit times at 115200 baud, in 84 MHz cycles, rounded up.
        assert_eq!(due, ps_to_cycles_ceil(f.bit_ps * 19 / 2, HZ));
        assert_eq!(b.receive_due(due - 1, &f, false), (None, 0));
        let (got, ovr) = b.receive_due(due, &f, false);
        assert_eq!(ovr, 0);
        assert_eq!(
            got,
            Some(Decoded {
                value: 0x5A,
                framing_error: false,
                parity_error: false
            })
        );
    }

    #[test]
    fn a_full_data_register_turns_the_next_character_into_an_overrun() {
        let (net, a, b) = pair(0, 0);
        let f = fmt_8n1(115_200);
        b.publish_format(f);
        a.transmit(0, 1, f);
        a.transmit(ps_to_cycles_ceil(f.frame_ps(), HZ), 2, f);
        let late = ps_to_cycles_ceil(3 * f.frame_ps(), HZ);
        let (got, ovr) = b.receive_due(late, &f, false);
        assert_eq!(got.map(|d| d.value), Some(1));
        assert_eq!(ovr, 1, "the second character found RXNE set");
        let r = net.report(0, 0);
        assert_eq!(r.links[0].directions[0].stats.overruns, 1);
        assert!(r.events.iter().any(|e| e.kind == NetEventKind::Overrun));
    }

    #[test]
    fn a_baud_mismatch_produces_framing_errors_not_clean_bytes() {
        let (net, a, b) = pair(0, 0);
        let tx = fmt_8n1(115_200);
        let rx = fmt_8n1(76_800);
        let frame = ps_to_cycles_ceil(tx.frame_ps(), HZ);
        let sent = [0xA5u16, 0x01, 0x00, 0x00, 0xA4, 0x55, 0x0F, 0xF0];
        for (i, v) in sent.iter().enumerate() {
            a.transmit(i as u64 * frame, *v, tx);
        }
        let mut got = Vec::new();
        while let Some(due) = b.next_due_cycle(&rx) {
            if let (Some(d), _) = b.receive_due(due, &rx, false) {
                got.push(d);
            }
        }
        let clean: Vec<u16> = got
            .iter()
            .filter(|d| !d.framing_error)
            .map(|d| d.value)
            .collect();
        assert_ne!(
            clean,
            sent.to_vec(),
            "a mismatched receiver must not read the bytes sent"
        );
        assert!(got.iter().any(|d| d.framing_error), "{got:?}");
        assert!(net.report(0, 0).links[0].directions[0].stats.framing_errors > 0);
    }

    #[test]
    fn parity_is_checked_by_the_receiver() {
        let (_net, a, b) = pair(0, 0);
        let mut even = fmt_8n1(115_200);
        even.parity = Parity::Even;
        let mut odd = even;
        odd.parity = Parity::Odd;
        a.transmit(0, 0x01, even);
        let due = b.next_due_cycle(&odd).unwrap();
        let (d, _) = b.receive_due(due, &odd, false);
        assert!(d.unwrap().parity_error);
    }

    #[test]
    fn delay_shifts_arrival_and_jitter_is_seeded() {
        let f = fmt_8n1(115_200);
        let (_n, a, b) = pair(1_000_000, 0);
        a.transmit(0, 7, f);
        let d0 = b.next_due_cycle(&f).unwrap();
        assert_eq!(d0, ps_to_cycles_ceil(1_000_000 + f.bit_ps * 19 / 2, HZ));

        let arrivals = |seed| {
            let net = TimedUartNet::new();
            let (_, a, b) = net.add_link(("a", "u", HZ), ("b", "u", HZ), 0, 5_000_000, seed);
            let mut out = Vec::new();
            for i in 0..8u64 {
                a.transmit(i * 20_000, i as u16, f);
                let due = b.next_due_cycle(&f).unwrap();
                out.push(due);
                let _ = b.receive_due(due, &f, false);
            }
            out
        };
        assert_eq!(arrivals(3), arrivals(3), "same seed, same jitter");
        assert_ne!(arrivals(3), arrivals(4), "different seed, different jitter");
    }

    #[test]
    fn a_cut_link_drops_and_truncates() {
        let (net, a, b) = pair(0, 0);
        let f = fmt_8n1(115_200);
        a.transmit(0, 0x00, f);
        // Cut after 3 bit times: start + 2 data bits made it; the rest reads 1.
        net.set_link_connected(0, false, f.bit_ps * 3);
        a.transmit(ps_to_cycles_ceil(f.frame_ps(), HZ), 0x11, f);
        let due = b.next_due_cycle(&f).unwrap();
        let (d, _) = b.receive_due(due, &f, false);
        assert_eq!(d.unwrap().value, 0xFC, "bits 2..7 read the idle line");
        let s = &net.report(0, 0).links[0].directions[0].stats;
        assert_eq!(s.truncated, 1);
        assert_eq!(s.dropped_link_cut, 1);
        assert!(b.next_due_cycle(&f).is_none());
    }

    #[test]
    fn lookahead_is_nine_and_a_half_bits_plus_delay() {
        let (net, a, b) = pair(2_000, 0);
        assert_eq!(net.lookahead_ps(), None, "nobody can transmit yet");
        let f = fmt_8n1(115_200);
        a.publish_format(f);
        b.publish_format(f);
        assert_eq!(net.lookahead_ps(), Some(2_000 + f.bit_ps * 19 / 2));
    }

    #[test]
    fn tagged_messages_report_origin_and_per_node_latency() {
        let (net, a, b) = pair(0, 0);
        net.set_message_tagging(Some(MessageTagging {
            sync: 0xA5,
            length: 4,
            id_offset: 1,
            id_bytes: 1,
            hop_offset: Some(2),
            checksum_xor: true,
        }));
        let f = fmt_8n1(115_200);
        let msg = [0xA5u8, 9, 1, 9 ^ 1];
        let frame = ps_to_cycles_ceil(f.frame_ps(), HZ);
        for (i, byte) in msg.iter().enumerate() {
            a.transmit(i as u64 * frame, u16::from(*byte), f);
            let due = b.next_due_cycle(&f).unwrap();
            let _ = b.receive_due(due, &f, false);
        }
        let r = net.report(0, 0);
        let m = &r.messages[0];
        assert_eq!(m.id, 9);
        assert_eq!(m.origin_node.as_deref(), Some("a"));
        assert_eq!(m.deliveries.len(), 1);
        assert_eq!(m.deliveries[0].hop, Some(1));
        let lat = m.deliveries[0].latency_ps.unwrap();
        // 3 whole characters then 9.5 bits of the 4th, give or take the
        // cycle rounding of each start.
        let expect = 3 * f.frame_ps() + f.bit_ps * 19 / 2;
        assert!(lat.abs_diff(expect) < 4 * 12_000, "{lat} vs {expect}");
    }
}
