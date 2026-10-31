// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Shared IEEE 802.15.4 (2.4-GHz O-QPSK, 250 kbit/s) air: the medium the
//! radios of one world exchange frames over.
//!
//! The same pattern as [`crate::peripherals::ble_air::BleAirBus`]: a shared
//! `Arc<Mutex<…>>` registry, isolated per bus, with a bounded TX trace. What
//! travels is the MPDU exactly as the transmitting radio built it (MHR,
//! payload and the 2-byte FCS), stamped with the world time of its first
//! and last bit.
//!
//! ## Faithfully modelled
//!
//! * **The frame.** [`AirFrame::mpdu`] is the transmitter's TX FIFO payload
//!   plus the FCS (CRC-16/KERMIT, IEEE 802.15.4-2006 §7.2.1.9) appended by
//!   the transmitter's AUTOCRC hardware.
//! * **Timing.** 32 µs per byte: SHR (4-byte preamble + SFD) + PHR + MPDU
//!   ([`airtime_ns`]). A receiver sees a frame only once the receiver's own
//!   time has passed the frame's last bit, and only if it was listening when
//!   the frame's first bit went out.
//! * **Broadcast.** Every registered radio on the channel sees every frame
//!   (per-receiver delivery mask), as on a real shared channel.
//! * **Hardware acknowledgments.** The CC2538 AUTOACK hardware decides from
//!   its own frame-filter settings alone (SWRU319C §23.9.8.3), so the air
//!   makes that decision for every registered receiver when the frame is
//!   pushed, from the receiver's [`RxFilter`], and queues the 5-byte ACK
//!   from that receiver 12 symbols (192 µs) after the frame ends
//!   (IEEE 802.15.4-2006 §7.5.6.4.2 aTurnaroundTime). Deciding at push time
//!   keeps the ACK on time no matter how far apart the nodes' simulated
//!   clocks are within their lockstep window.
//! * **Frame filtering** ([`filter`]): SWRU319C §23.9.5.1, length, reserved
//!   FCF bits, frame version, address modes, destination PAN/short/extended
//!   address and the per-frame-type rules.
//!
//! ## Idealised
//!
//! * Lossless and collision-free: no path loss, range, interference or
//!   capture. Two overlapping frames are both delivered. Every node hears
//!   every other node (the Renode `cc2538_rpl-udp` topology puts the two
//!   clients out of each other's range; here they hear each other too).
//! * The ACK decision uses the receiver's filter settings at push time, not
//!   at the frame's end, and the acknowledging radio's own FSM does not show
//!   the ACK transmission.
//! * `MODIFY_FT_FILTER`, source matching and AUTOPEND are not modelled; the
//!   ACK frame-pending bit is `FRMCTRL1.PENDING_OR` only.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// One byte on the air: 2 symbols of 16 µs.
pub const BYTE_NS: u64 = 32_000;
/// SHR (4-byte preamble + 1-byte SFD) plus the 1-byte PHR.
pub const SHR_PHR_BYTES: u64 = 6;
/// SFD fully received: preamble + SFD.
pub const SFD_NS: u64 = 5 * BYTE_NS;
/// aTurnaroundTime, 12 symbols: TX after an RX ends (the ACK delay).
pub const TURNAROUND_NS: u64 = 192_000;
/// Frames kept on the air before the oldest is dropped.
const AIR_DEPTH: usize = 256;
/// Frames kept in the inspection trace.
const TX_HISTORY_CAP: usize = 200;
/// Radios per air (one bit each in a frame's delivery mask).
pub const MAX_NODES: usize = 64;

/// On-air duration of an MPDU of `mpdu_len` bytes (FCS included).
pub fn airtime_ns(mpdu_len: usize) -> u64 {
    (SHR_PHR_BYTES + mpdu_len as u64) * BYTE_NS
}

/// IEEE 802.15.4 FCS: CRC-16/KERMIT (ITU-T polynomial, reflected, init 0),
/// transmitted low byte first.
pub fn fcs(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// A receiver's frame-filter and AUTOACK settings (FRMFILT0/1, FRMCTRL0/1,
/// the FFSM local address), plus whether it is listening and on which
/// channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RxFilter {
    pub listening: bool,
    /// Channel key: the radio's frequency setting (CC2538 `FREQCTRL`).
    pub channel: u32,
    /// FRMFILT0: FRM_FILTER_EN, PAN_COORDINATOR, MAX_FRAME_VERSION,
    /// FCF_RESERVED_MASK.
    pub frame_filter: bool,
    pub pan_coordinator: bool,
    pub max_frame_version: u8,
    pub fcf_reserved_mask: u8,
    /// FRMFILT1 ACCEPT_FT_0..3 (bit 0 = beacon) and ACCEPT_FT_4TO7 (bit 4).
    pub accept_types: u8,
    pub autoack: bool,
    /// FRMCTRL1.PENDING_OR: every ACK carries frame-pending.
    pub ack_pending: bool,
    pub pan_id: u16,
    pub short_addr: u16,
    /// Extended address as the over-the-air (little-endian) byte order.
    pub ext_addr: [u8; 8],
}

/// Outcome of [`filter`] for one MPDU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterVerdict {
    pub accepted: bool,
    /// AUTOACK would acknowledge it (§23.9.8.3).
    pub ack: bool,
}

/// SWRU319C §23.9.5.1 frame filtering for `mpdu` (FCS included), and the
/// §23.9.8.3 AUTOACK decision.
pub fn filter(f: &RxFilter, mpdu: &[u8]) -> FilterVerdict {
    let reject = FilterVerdict {
        accepted: false,
        ack: false,
    };
    let len = mpdu.len();
    let crc_ok = len >= 2 && {
        let (body, tail) = mpdu.split_at(len - 2);
        fcs(body) == u16::from_le_bytes([tail[0], tail[1]])
    };
    if !f.frame_filter {
        return FilterVerdict {
            accepted: true,
            ack: false,
        };
    }
    if len < 5 {
        return reject;
    }
    let fcf = u16::from_le_bytes([mpdu[0], mpdu[1]]);
    let ftype = (fcf & 0x7) as u8;
    let ack_request = fcf & (1 << 5) != 0;
    let pan_comp = fcf & (1 << 6) != 0;
    let reserved = ((fcf >> 7) & 0x7) as u8;
    let dst_mode = (fcf >> 10) & 0x3;
    let version = ((fcf >> 12) & 0x3) as u8;
    let src_mode = (fcf >> 14) & 0x3;
    if reserved & f.fcf_reserved_mask != 0 || version > f.max_frame_version {
        return reject;
    }
    if dst_mode == 1 || src_mode == 1 {
        return reject;
    }
    let addr_len = |mode: u16| match mode {
        2 => 2,
        3 => 8,
        _ => 0,
    };
    let dst_pan_len = if dst_mode != 0 { 2 } else { 0 };
    let src_pan_len = if src_mode != 0 && !(pan_comp && dst_mode != 0) {
        2
    } else {
        0
    };
    let min_len = 3 + dst_pan_len + addr_len(dst_mode) + src_pan_len + addr_len(src_mode) + 2;
    if len < min_len {
        return reject;
    }
    let mut p = 3;
    if dst_mode != 0 {
        let pan = u16::from_le_bytes([mpdu[p], mpdu[p + 1]]);
        p += 2;
        if pan != f.pan_id && pan != 0xFFFF {
            return reject;
        }
        if dst_mode == 2 {
            let a = u16::from_le_bytes([mpdu[p], mpdu[p + 1]]);
            p += 2;
            if a != f.short_addr && a != 0xFFFF {
                return reject;
            }
        } else {
            if mpdu[p..p + 8] != f.ext_addr {
                return reject;
            }
            p += 8;
        }
    }
    let src_pan = if src_pan_len == 2 {
        Some(u16::from_le_bytes([mpdu[p], mpdu[p + 1]]))
    } else {
        None
    };
    let accepts = |bit: u8| f.accept_types & (1 << bit) != 0;
    let ok = match ftype {
        0 => {
            accepts(0)
                && len >= 9
                && dst_mode == 0
                && src_mode >= 2
                && (src_pan == Some(f.pan_id) || f.pan_id == 0xFFFF)
        }
        1 | 3 => {
            accepts(if ftype == 1 { 1 } else { 3 })
                && len >= 9
                && (dst_mode != 0
                    || (src_mode != 0 && f.pan_coordinator && src_pan == Some(f.pan_id)))
        }
        2 => accepts(2) && len == 5,
        _ => accepts(4) && len >= 9,
    };
    FilterVerdict {
        accepted: ok,
        ack: ok && f.autoack && ack_request && ftype != 0 && ftype != 2 && crc_ok,
    }
}

/// One frame on the air.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AirFrame {
    /// Monotonic within the air.
    pub seq: u64,
    /// Transmitting radio (its [`Ieee802154AirBus::register`] index).
    pub source: usize,
    pub channel: u32,
    /// MHR + payload + FCS.
    pub mpdu: Vec<u8>,
    /// First bit (preamble) and last bit, in world ns.
    pub start_ns: u64,
    pub end_ns: u64,
    #[serde(skip)]
    seen: u64,
}

#[derive(Debug, Default)]
struct Air {
    nodes: Vec<RxFilter>,
    frames: VecDeque<AirFrame>,
    next_seq: u64,
    history: VecDeque<AirFrame>,
}

impl Air {
    fn push(&mut self, source: usize, channel: u32, mpdu: Vec<u8>, start_ns: u64) -> AirFrame {
        let frame = AirFrame {
            seq: self.next_seq,
            source,
            channel,
            end_ns: start_ns + airtime_ns(mpdu.len()),
            mpdu,
            start_ns,
            seen: 1 << source,
        };
        self.next_seq += 1;
        self.frames.push_back(frame.clone());
        while self.frames.len() > AIR_DEPTH {
            self.frames.pop_front();
        }
        self.history.push_back(frame.clone());
        while self.history.len() > TX_HISTORY_CAP {
            self.history.pop_front();
        }
        frame
    }
}

/// A shared 802.15.4 air. Radios holding clones of one bus hear each other;
/// separate buses are isolated.
#[derive(Debug, Clone, Default)]
pub struct Ieee802154AirBus {
    inner: Arc<Mutex<Air>>,
}

impl Ieee802154AirBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a radio; returns its index, or `None` past [`MAX_NODES`].
    pub fn register(&self) -> Option<usize> {
        let mut air = self.inner.lock().ok()?;
        if air.nodes.len() >= MAX_NODES {
            return None;
        }
        air.nodes.push(RxFilter::default());
        Some(air.nodes.len() - 1)
    }

    /// Publish a radio's current receive settings (used for the AUTOACK
    /// decision on frames pushed later).
    pub fn set_filter(&self, node: usize, f: &RxFilter) {
        if let Ok(mut air) = self.inner.lock() {
            if let Some(slot) = air.nodes.get_mut(node) {
                if slot != f {
                    *slot = f.clone();
                }
            }
        }
    }

    /// Put `mpdu` (FCS included) on `channel` from `node`, its first bit at
    /// `start_ns`. Every other listening radio on the channel whose filter
    /// accepts it with AUTOACK answers with an ACK 192 µs after it ends.
    /// Returns the frame's end time.
    pub fn transmit(&self, node: usize, channel: u32, mpdu: Vec<u8>, start_ns: u64) -> u64 {
        let Ok(mut air) = self.inner.lock() else {
            return start_ns + airtime_ns(mpdu.len());
        };
        let acks: Vec<(usize, bool)> = air
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, f)| *i != node && f.listening && f.channel == channel)
            .filter(|(_, f)| filter(f, &mpdu).ack)
            .map(|(i, f)| (i, f.ack_pending))
            .collect();
        let seqno = mpdu.get(2).copied().unwrap_or(0);
        let frame = air.push(node, channel, mpdu, start_ns);
        for (acker, pending) in acks {
            let mut ack = vec![0x02, 0x00, seqno];
            if pending {
                ack[0] |= 1 << 4;
            }
            let c = fcs(&ack);
            ack.extend_from_slice(&c.to_le_bytes());
            air.push(acker, channel, ack, frame.end_ns + TURNAROUND_NS);
        }
        frame.end_ns
    }

    /// Frames for `node` that ended by `now_ns`, oldest end first, each
    /// returned once. Frames that started before `listen_since_ns` (the
    /// radio was not yet listening) are consumed without being returned.
    pub fn take_received(&self, node: usize, listen_since_ns: u64, now_ns: u64) -> Vec<AirFrame> {
        let Ok(mut air) = self.inner.lock() else {
            return Vec::new();
        };
        let bit = 1u64 << node;
        let mut out = Vec::new();
        for f in air.frames.iter_mut() {
            if f.seen & bit != 0 || f.end_ns > now_ns {
                continue;
            }
            f.seen |= bit;
            if f.start_ns >= listen_since_ns {
                out.push(f.clone());
            }
        }
        let all = if air.nodes.len() >= 64 {
            u64::MAX
        } else {
            (1u64 << air.nodes.len()) - 1
        };
        air.frames.retain(|f| f.seen & all != all);
        out.sort_by_key(|f| (f.end_ns, f.seq));
        out
    }

    /// The first frame from another radio on `channel` that is on the air
    /// at `now_ns`, as `(start_ns, end_ns)`.
    pub fn on_air(&self, node: usize, channel: u32, now_ns: u64) -> Option<(u64, u64)> {
        let air = self.inner.lock().ok()?;
        air.frames
            .iter()
            .filter(|f| f.source != node && f.channel == channel)
            .find(|f| f.start_ns <= now_ns && now_ns < f.end_ns)
            .map(|f| (f.start_ns, f.end_ns))
    }

    /// The most recent frames, oldest first (inspection / tests).
    pub fn trace_snapshot(&self) -> Vec<AirFrame> {
        self.inner
            .lock()
            .map(|a| a.history.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_fcs(mut body: Vec<u8>) -> Vec<u8> {
        let c = fcs(&body);
        body.extend_from_slice(&c.to_le_bytes());
        body
    }

    fn node(ext: [u8; 8]) -> RxFilter {
        RxFilter {
            listening: true,
            channel: 0x0B,
            frame_filter: true,
            max_frame_version: 1,
            fcf_reserved_mask: 0,
            accept_types: 0x0F,
            autoack: true,
            pan_id: 0xABCD,
            short_addr: 0xFFFF,
            ext_addr: ext,
            ..Default::default()
        }
    }

    /// Data frame, PAN-ID compressed, extended dst and src, AR set.
    fn unicast(dst: [u8; 8], src: [u8; 8], seq: u8, ar: bool) -> Vec<u8> {
        let mut fcf: u16 = 0x0001 | (1 << 6) | (3 << 10) | (1 << 12) | (3 << 14);
        if ar {
            fcf |= 1 << 5;
        }
        let mut b = fcf.to_le_bytes().to_vec();
        b.push(seq);
        b.extend_from_slice(&0xABCDu16.to_le_bytes());
        b.extend_from_slice(&dst);
        b.extend_from_slice(&src);
        b.extend_from_slice(b"payload");
        with_fcs(b)
    }

    #[test]
    fn fcs_matches_the_802_15_4_check_value() {
        // CRC-16/KERMIT check value over "123456789".
        assert_eq!(fcs(b"123456789"), 0x2189);
    }

    #[test]
    fn filter_accepts_own_ext_address_and_broadcast_and_acks_only_unicast() {
        let me = [1, 0, 0, 0, 0, 0, 0, 0];
        let other = [2, 0, 0, 0, 0, 0, 0, 0];
        let f = node(me);
        let v = filter(&f, &unicast(me, other, 7, true));
        assert!(v.accepted && v.ack);
        let v = filter(&f, &unicast(other, me, 7, true));
        assert!(!v.accepted && !v.ack, "someone else's frame");
        // Broadcast: short dst 0xFFFF, src extended, no AR.
        let fcf: u16 = 0x0001 | (1 << 6) | (2 << 10) | (1 << 12) | (3 << 14);
        let mut b = fcf.to_le_bytes().to_vec();
        b.push(9);
        b.extend_from_slice(&0xABCDu16.to_le_bytes());
        b.extend_from_slice(&0xFFFFu16.to_le_bytes());
        b.extend_from_slice(&other);
        b.extend_from_slice(b"dio");
        let v = filter(&f, &with_fcs(b));
        assert!(v.accepted && !v.ack);
        // A corrupted FCS is still accepted into the FIFO but not acked.
        let mut bad = unicast(me, other, 7, true);
        let n = bad.len();
        bad[n - 1] ^= 0xFF;
        let v = filter(&f, &bad);
        assert!(v.accepted && !v.ack);
        // Wrong PAN.
        let mut g = f.clone();
        g.pan_id = 0x1234;
        assert!(!filter(&g, &unicast(me, other, 7, true)).accepted);
        // ACK frames: length 5, accepted while ACCEPT_FT_2_ACK is set.
        let ack = with_fcs(vec![0x02, 0x00, 7]);
        assert!(filter(&f, &ack).accepted);
        g = f.clone();
        g.accept_types &= !(1 << 2);
        assert!(!filter(&g, &ack).accepted);
        // Filter disabled: everything, and no AUTOACK.
        g = f.clone();
        g.frame_filter = false;
        let v = filter(&g, &unicast(other, me, 7, true));
        assert!(v.accepted && !v.ack);
    }

    #[test]
    fn frames_broadcast_on_time_and_the_addressee_acks_192us_after() {
        let air = Ieee802154AirBus::new();
        let a = air.register().unwrap();
        let b = air.register().unwrap();
        let c = air.register().unwrap();
        let ext = |i: u8| [i, 0, 0, 0, 0, 0, 0, 0];
        air.set_filter(a, &node(ext(1)));
        air.set_filter(b, &node(ext(2)));
        air.set_filter(c, &node(ext(3)));
        let mpdu = unicast(ext(2), ext(1), 42, true);
        let end = air.transmit(a, 0x0B, mpdu.clone(), 1_000_000);
        assert_eq!(end, 1_000_000 + airtime_ns(mpdu.len()));
        // Not received before its last bit.
        assert!(air.take_received(b, 0, end - 1).is_empty());
        assert_eq!(air.on_air(b, 0x0B, end - 1), Some((1_000_000, end)));
        assert_eq!(air.on_air(a, 0x0B, end - 1), None, "own frame");
        assert_eq!(air.on_air(b, 0x0C, end - 1), None, "other channel");
        let rx = air.take_received(b, 0, end);
        assert_eq!(rx.len(), 1);
        assert_eq!(rx[0].mpdu, mpdu);
        assert!(air.take_received(b, 0, end).is_empty(), "delivered once");
        // c hears the data frame too, then b's ACK.
        let ack_end = end + TURNAROUND_NS + airtime_ns(5);
        let rx = air.take_received(c, 0, ack_end);
        assert_eq!(rx.len(), 2);
        assert_eq!(rx[1].source, b);
        // The sender gets exactly the ACK, from b, on time.
        let rx = air.take_received(a, 0, ack_end);
        assert_eq!(rx.len(), 1);
        assert_eq!(rx[0].mpdu, with_fcs(vec![0x02, 0x00, 42]));
        assert_eq!(rx[0].start_ns, end + TURNAROUND_NS);
    }

    #[test]
    fn a_radio_not_listening_at_the_first_bit_misses_the_frame() {
        let air = Ieee802154AirBus::new();
        let a = air.register().unwrap();
        let b = air.register().unwrap();
        let end = air.transmit(a, 0x0B, with_fcs(vec![0x02, 0, 1]), 100);
        assert!(air.take_received(b, 101, end).is_empty());
        assert!(
            air.take_received(b, 0, end).is_empty(),
            "consumed, not re-delivered"
        );
    }

    #[test]
    fn no_ack_from_a_radio_that_is_off_or_on_another_channel() {
        let air = Ieee802154AirBus::new();
        let a = air.register().unwrap();
        let b = air.register().unwrap();
        let ext = |i: u8| [i, 0, 0, 0, 0, 0, 0, 0];
        let mut fb = node(ext(2));
        fb.listening = false;
        air.set_filter(b, &fb);
        let end = air.transmit(a, 0x0B, unicast(ext(2), ext(1), 1, true), 0);
        assert!(air.take_received(a, 0, end + 10_000_000).is_empty());
        fb.listening = true;
        fb.channel = 0x10;
        air.set_filter(b, &fb);
        let end = air.transmit(a, 0x0B, unicast(ext(2), ext(1), 2, true), end);
        assert!(air.take_received(a, 0, end + 10_000_000).is_empty());
    }
}
