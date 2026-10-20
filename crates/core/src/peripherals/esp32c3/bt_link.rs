// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The RW-BLE core's **timed** radio events: everything that needs a request
//! and a response 150 µs apart inside one programmed event.
//!
//! [`super`] runs a programmed event as "transmit at the start, pick up what
//! the air carries, end after the programmed duration". That is enough for
//! advertising and passive scanning and it is left exactly as it was for them.
//! It cannot carry a connection: a connection event is an exchange, and the
//! link layer above the core (the ROM's `lld_con`, `lld_adv`, `lld_init`)
//! expects the core to have done the exchange — acknowledged, retransmitted,
//! chosen the data channel, timestamped each sync — by the time it looks.
//!
//! This module is that exchange. It is dispatched on the control-structure
//! format (`CS+0x00` bits[4:0]; the codes are the RW-BLE `EM_BLE_CS_FMT_*`
//! values, and the two the twin had already measured — `0x04` advertising,
//! `0x08` passive scan — are exactly where that enumeration puts them):
//!
//! | format | activity | what the core does here |
//! |---|---|---|
//! | `0x02` | master connection | TX at the anchor; RX the slave's reply at T_IFS; repeat while either side sets MD |
//! | `0x03` | slave connection | RX the master's packet inside the RX window; TX the reply at T_IFS; repeat while MD |
//! | `0x04` | connectable advertising | TX `ADV_IND` (as before); then RX at T_IFS: `SCAN_REQ` → TX `SCAN_RSP`, `CONNECT_IND` → report and end |
//! | `0x09` | active scan | RX advertising; for a scannable PDU TX `SCAN_REQ` and RX the `SCAN_RSP` |
//! | `0x0E` | initiator | RX the target's `ADV_IND`; TX `CONNECT_IND` at T_IFS with the window offset filled in |
//!
//! ## Time
//!
//! Frames carry the air time of their first bit ([`BleAirFrame::air_ns`]). A
//! receive window is a range of *start* times. The core decides that nothing
//! came only once its own clock is [`RX_DECISION_LAG_NS`] past the window, so
//! a peer node that the world has stepped up to one quantum behind this one
//! has had the chance to transmit. A reply is always stamped at
//! `request_end + T_IFS` exactly, however late within that lag the core
//! noticed the request — so the exchange on the air is the same no matter how
//! the host interleaved the two nodes. **A world that steps its nodes further
//! apart than the lag breaks this**, which is why `World` steps in time quanta
//! when a BLE link is possible.
//!
//! ## Field layouts
//!
//! RW-BLE register headers of the same IP generation as the C3 (90-byte
//! control structure, 14-byte TX descriptor, 20-byte RX descriptor — the sizes
//! the C3 ROM indexes with) name every field used here; see the constants.
//! Where the software contract matters it is the one the reference RW-BLE
//! link layer (`lld_con_rx`, `lld_con_tx`, `lld_adv_pkt_rx`,
//! `lld_init_process_pkt_rx/tx`) reads back, and each such use was checked
//! against the C3 mask ROM (`esp32c3_rev3_rom.elf`), e.g. `r_lld_con_frm_isr`
//! reading `CS+0x56` and `TXRXCNTL.LASTEMPTY`, and
//! `r_lld_init_process_pkt_tx_cal_con_timestamp` reading `CS+0x44`.
//!
//! ## Idealised, stated plainly
//!
//! No PHY: no CRC, no bit errors, no encryption (a link that turns on
//! encryption is not supported — `LL_ENC_REQ` would be answered by firmware
//! and then every packet would need a MIC). 1M PHY only. Whitelist and
//! resolvable-private-address filtering are not applied (the filter policy is
//! read by nothing here). One frame per advertising event, on the hop channel
//! the control structure names — the same idealisation the advertising path
//! already makes.

use super::*;
use crate::peripherals::ble_air::{airtime_ns, SYNC_OFFSET_NS, T_IFS_NS};

// ── Control-structure fields (RW-BLE `reg_em_ble_cs.h`, 90-byte layout) ─────

/// `CS+0x02` `LINK`: bits[4:0] link label. (Read by nothing here: the label
/// the core stamps is the control-structure index, as the scan path found.)
/// `CS+0x14` `FILTPOL_RALCNTL`.
/// `CS+0x18` `TXRXCNTL`: bit15 `RXBUFF_FULL`, bit14 `LASTEMPTY`, bit13 `SN`,
/// bit12 `NESN`. The core owns the acknowledgement state and keeps it here.
const CS_TXRXCNTL: u32 = 0x18;
const TXRX_RXBUFF_FULL: u16 = 1 << 15;
const TXRX_LASTEMPTY: u16 = 1 << 14;
const TXRX_SN: u16 = 1 << 13;
const TXRX_NESN: u16 = 1 << 12;
/// `CS+0x1A` `RXWINCNTL`: bit15 `RXWIDE` (size in 625 µs slots, else µs),
/// bits[14:0] the half receive-window size.
const CS_RXWINCNTL: u32 = 0x1A;
/// `CS+0x1E` `WINOFFSET` (initiator): the transmit-window offset, in 1.25 ms
/// units, valid at the event start (`lld_init_compute_winoffset`).
const CS_WINOFFSET: u32 = 0x1E;
/// `CS+0x22` `CONNINTERVAL` (initiator) / `LLCHMAP0` (connection).
const CS_CONNINTERVAL: u32 = 0x22;
const CS_CHMAP0: u32 = 0x22;
/// `CS+0x2C..0x31` `ADV_BD_ADDR`: the initiator's target address.
const CS_ADV_BD_ADDR: u32 = 0x2C;
/// `CS+0x44` `TXWINOFFSET`: the window offset the core actually put in the
/// `CONNECT_IND`. `r_lld_init_process_pkt_tx_cal_con_timestamp` reads it
/// (`lhu 68(cs)`, `slli 2`) to place the first master anchor.
const CS_TXWINOFFSET: u32 = 0x44;
/// `CS+0x50` `EVTCNT`: the connection event counter software writes per event.
const CS_EVTCNT: u32 = 0x50;
/// `CS+0x56` `TXRXDESCCNT`: bits[15:8] RX descriptors used, bits[7:0] TX
/// descriptors acknowledged, in the event. `r_lld_con_frm_isr` reads the RX
/// count (`lhu 86(cs)`, `>> 8`).
const CS_TXRXDESCCNT: u32 = 0x56;

/// `HOPCNTL` (`CS+0x16`) bit14 `HOP_SEL`: channel selection algorithm #2.
const HOP_SEL_CSA2: u16 = 1 << 14;
/// `HOPCNTL` bit15 `FH_EN`: the core hops (CSA #1 adds `HOP_INT`, [12:8]).
const HOP_FH_EN: u16 = 1 << 15;
/// `HOPCNTL` bits[5:0] `CH_IDX`.
const HOP_CH_IDX: u16 = 0x3F;

// ── TX descriptor (`reg_em_ble_tx_desc.h`, 14 bytes) ─────────────────────────

/// `+0x0` `TXCNTL`: bit15 `TXDONE`, bits[14:0] `NEXTPTR`.
const TXD_CNTL: u32 = 0x0;
const TXD_DONE: u16 = 0x8000;
/// Data-PDU header bits in `TXPHCE` (`+0x2`): `LLID` [1:0], `NESN` 2, `SN` 3,
/// `MD` 4; length in [15:8]. The core fills `SN`/`NESN`, firmware `LLID`/`MD`.
const PH_LLID: u8 = 0x03;
const PH_NESN: u8 = 1 << 2;
const PH_SN: u8 = 1 << 3;
const PH_MD: u8 = 1 << 4;

// ── RX descriptor status words (`reg_em_ble_rx_desc.h`) ──────────────────────

/// `RXSTATCE` (connection) — no error. Unlike the advertising status the scan
/// path writes (`0x0040`, `BDADDR_MATCH`), bit 6 here is `SN_ERR`, so a good
/// data packet must read 0.
const RXSTAT_CE_OK: u16 = 0;
/// `RXSTATCE.SN_ERR`: a retransmission of a packet already received.
/// `lld_con_rx` drops the payload on it.
const RXSTAT_CE_SN_ERR: u16 = 1 << 6;
/// `RXSTATADV.BDADDR_MATCH`: the good-reception word for advertising-channel
/// PDUs, the same `0x0040` the scan path already writes.
const RXSTAT_ADV_OK: u16 = 0x0040;
/// RX descriptor `+0x6` `RXCHASS`: bits[13:8] the channel the packet arrived
/// on, [7:0] RSSI (0: no PHY), [15:14] rate (0: 1M).
const RXD_CHASS: u32 = 0x6;

// ── Advertising PDU types ────────────────────────────────────────────────────
const PDU_ADV_IND: u8 = 0x0;
const PDU_ADV_DIRECT_IND: u8 = 0x1;
const PDU_SCAN_REQ: u8 = 0x3;
const PDU_SCAN_RSP: u8 = 0x4;
const PDU_CONNECT_IND: u8 = 0x5;
const PDU_ADV_SCAN_IND: u8 = 0x6;

// ── Format codes (`EM_BLE_CS_FMT_*`) ─────────────────────────────────────────
const FMT_MASTER: u16 = 0x02;
const FMT_SLAVE: u16 = 0x03;
const FMT_LDC_ADV: u16 = 0x04;
const FMT_ACTIVE_SCAN: u16 = 0x09;
const FMT_INITIATOR: u16 = 0x0E;

/// How far past a receive window the core waits before concluding nothing
/// arrived. It must cover the furthest a world lets two nodes' clocks drift
/// apart (one stepping quantum); see the module docs.
pub const RX_DECISION_LAG_NS: u64 = 50_000;
/// Poll cadence while a wide receive window is open (a slave's first window,
/// a scan, an initiator). Only affects how soon the core notices, never what
/// it stamps.
const POLL_NS: u64 = 20_000;
/// Tolerance on "the reply starts exactly T_IFS after": both sides stamp from
/// the same arithmetic, so this only absorbs the ns rounding of the cycle
/// conversion.
const IFS_TOLERANCE_NS: u64 = 2_000;
/// The C3 CPU clock the whole model is pinned to (see
/// [`super::CYCLES_PER_CLKN_TICK`]): 160 MHz, i.e. 25/4 ns per cycle.
const NS_PER_CYCLE_NUM: u64 = 25;
const NS_PER_CYCLE_DEN: u64 = 4;
/// Packets per connection event before the core closes it, whatever MD says.
/// Guards the model against a peer that sets MD forever; firmware bounds the
/// event with its duration as well.
const MAX_PACKETS_PER_EVENT: u32 = 16;

/// What the in-flight timed event is doing.
#[derive(Debug, Clone)]
pub(super) enum LinkStep {
    /// Advertising: `ADV_IND` sent, listening at T_IFS for a request.
    AdvListen { adv_end_ns: u64 },
    /// Connection: waiting for the peer's packet to start inside
    /// `[from_ns, to_ns]`.
    ConRx { from_ns: u64, to_ns: u64 },
    /// Active scan / initiator: receive window open until the event ends.
    ScanRx { from_ns: u64 },
    /// Active scan: `SCAN_REQ` sent, waiting for the `SCAN_RSP`.
    ScanRspRx { at_ns: u64 },
    /// Nothing more to do but end the event at `at` (elapsed cycles).
    EndAt,
}

/// The in-flight timed event.
#[derive(Debug, Clone)]
pub(super) struct LinkCtx {
    pub(super) step: LinkStep,
    format: u16,
    cs: u32,
    link_label: u16,
    channel: u8,
    access_address: u32,
    crc_init: u32,
    /// The event's hard end (elapsed cycles): `ET` start + duration.
    event_end: u64,
    /// Next thing to do, elapsed cycles.
    pub(super) deadline: u64,
    /// Connection: packets exchanged so far in this event.
    packets: u32,
    rx_desc_used: u16,
    tx_desc_acked: u16,
    /// Connection: MD bit of the peer's last packet in this event.
    peer_md: bool,
}

impl Esp32c3Bt {
    // ── Time ────────────────────────────────────────────────────────────────

    /// Air time (ns since the world started) of elapsed-cycle `elapsed`.
    pub(super) fn air_ns_at(&self, elapsed: u64) -> u64 {
        (elapsed + self.clock_base.unwrap_or(0)) * NS_PER_CYCLE_NUM / NS_PER_CYCLE_DEN
    }

    /// Earliest elapsed cycle at or after air time `ns`.
    fn elapsed_for_ns(&self, ns: u64) -> u64 {
        (ns * NS_PER_CYCLE_DEN)
            .div_ceil(NS_PER_CYCLE_NUM)
            .saturating_sub(self.clock_base.unwrap_or(0))
    }

    /// CLKN (half-slot count) and the down-counting fine value
    /// (`624 - half-µs into the slot`) of air time `ns` — the pair an RX
    /// descriptor's sync timestamp and a control structure's timers use.
    fn clkn_fine_of_ns(&self, ns: u64) -> (u32, u16) {
        let elapsed = self.elapsed_for_ns(ns);
        (Self::clkn_at(elapsed), Self::fine_at(elapsed) as u16)
    }

    // ── Dispatch ────────────────────────────────────────────────────────────

    /// Start a programmed event on the timed path if its format is one this
    /// module owns. Returns `false` to leave it to the untimed path in
    /// [`super`] (passive scan, and anything unknown).
    pub(super) fn link_start(
        &mut self,
        bus: &mut dyn Bus,
        et_idx: u32,
        start: u64,
        duration: u64,
    ) -> bool {
        let et = Self::et_entry(et_idx);
        let Some(cs_ptr) = self.em_read_u16(bus, et + ET_CS_PTR) else {
            return false;
        };
        let cs = u32::from(cs_ptr) * 2;
        let Some(format_word) = self.em_read_u16(bus, cs + CS_FORMAT) else {
            return false;
        };
        let format = format_word & 0x1F;
        if !matches!(
            format,
            FMT_MASTER | FMT_SLAVE | FMT_LDC_ADV | FMT_ACTIVE_SCAN | FMT_INITIATOR
        ) {
            return false;
        }
        let Some(cs_rel) = cs.checked_sub(EM_CS_OFFSET) else {
            return false;
        };
        let link_label = ((cs_rel / CS_STRIDE) as u16) & 0x1F;
        let access_address = self.em_read_u32(bus, cs + CS_ACCESS_ADDR).unwrap_or(0);
        let crc_init = self.em_read_u32(bus, cs + CS_CRC_INIT).unwrap_or(0) & 0x00FF_FFFF;
        let hop = self.em_read_u16(bus, cs + CS_HOP_CTRL).unwrap_or(0);
        let mut ctx = LinkCtx {
            step: LinkStep::EndAt,
            format,
            cs,
            link_label,
            channel: (hop & CS_CHANNEL_MASK) as u8,
            access_address,
            crc_init,
            event_end: start + duration,
            deadline: start + duration,
            packets: 0,
            rx_desc_used: 0,
            tx_desc_acked: 0,
            peer_md: false,
        };
        let start_ns = self.air_ns_at(start);
        match format {
            FMT_LDC_ADV => {
                // The advertising PDU goes out exactly as before, now stamped.
                let Some(pdu) = self.transmit_event_at(bus, et_idx, start_ns) else {
                    return false;
                };
                let pdu_type = pdu[0] & 0x0F;
                if matches!(
                    pdu_type,
                    PDU_ADV_IND | PDU_ADV_SCAN_IND | PDU_ADV_DIRECT_IND
                ) {
                    let adv_end_ns = start_ns + airtime_ns(pdu.len());
                    ctx.step = LinkStep::AdvListen { adv_end_ns };
                    ctx.deadline = self.elapsed_for_ns(
                        adv_end_ns + T_IFS_NS + IFS_TOLERANCE_NS + RX_DECISION_LAG_NS,
                    );
                }
            }
            FMT_SLAVE | FMT_MASTER => {
                ctx.channel = self.data_channel(bus, cs, hop);
                let _ = self.em_write_u16(bus, cs + CS_TXRXDESCCNT, 0);
                if format == FMT_SLAVE {
                    let win = self.em_read_u16(bus, cs + CS_RXWINCNTL).unwrap_or(0);
                    let half_ns = if win & 0x8000 != 0 {
                        u64::from(win & 0x7FFF) * 625_000
                    } else {
                        u64::from(win & 0x7FFF) * 1_000
                    };
                    // The master's packet starts somewhere in
                    // `[start, start + 2 * half window]`; allow a small
                    // guard either side for the ns rounding.
                    let to_ns =
                        (start_ns + 2 * half_ns + 32_000).min(self.air_ns_at(ctx.event_end));
                    ctx.step = LinkStep::ConRx {
                        from_ns: start_ns.saturating_sub(IFS_TOLERANCE_NS),
                        to_ns,
                    };
                    ctx.deadline = start + 1;
                    if bt_trace_enabled() {
                        let nid = self.node_id;
                        eprintln!(
                            "[bt{nid}] con S event ch{} hop={hop:#06x} aa={:#010x} win={win:#06x} rx {}..{} ns evtcnt={}",
                            ctx.channel,
                            ctx.access_address,
                            start_ns,
                            to_ns,
                            self.em_read_u16(bus, cs + CS_EVTCNT).unwrap_or(0)
                        );
                    }
                } else {
                    // Master: the anchor is the event start.
                    self.link = Some(ctx);
                    self.con_transmit(bus, start_ns);
                    return true;
                }
            }
            FMT_ACTIVE_SCAN | FMT_INITIATOR => {
                ctx.step = LinkStep::ScanRx { from_ns: start_ns };
                ctx.deadline = start + 1;
            }
            _ => unreachable!(),
        }
        self.link = Some(ctx);
        true
    }

    /// Advance the timed event. Returns the next deadline, or `None` when the
    /// event is over (the caller writes the end status and raises
    /// `sch_prog_end`).
    pub(super) fn link_service(&mut self, bus: &mut dyn Bus, elapsed: u64) -> Option<u64> {
        let ctx = self.link.as_ref()?.clone();
        let now_ns = self.air_ns_at(elapsed);
        match ctx.step {
            LinkStep::EndAt => None,
            LinkStep::AdvListen { adv_end_ns } => {
                let from = adv_end_ns + T_IFS_NS - IFS_TOLERANCE_NS;
                let to = adv_end_ns + T_IFS_NS + IFS_TOLERANCE_NS;
                let frame = self.air.receive_window(
                    ctx.channel,
                    ctx.access_address,
                    from,
                    to,
                    self.node_id,
                );
                if let Some(f) = frame {
                    let end = f.end_ns().unwrap_or(now_ns);
                    if end > now_ns {
                        return Some(self.elapsed_for_ns(end));
                    }
                    if let Some(next) = self.adv_on_request(bus, &ctx, &f) {
                        return Some(next);
                    }
                }
                // Nothing for us: the event runs out its programmed duration.
                self.set_link_step(LinkStep::EndAt);
                Some(ctx.event_end.max(elapsed))
            }
            LinkStep::ConRx { from_ns, to_ns } => {
                let frame = self.air.receive_window(
                    ctx.channel,
                    ctx.access_address,
                    from_ns,
                    to_ns.min(now_ns),
                    self.node_id,
                );
                if let Some(f) = frame {
                    let end = f.end_ns().unwrap_or(now_ns);
                    if end > now_ns {
                        return Some(self.elapsed_for_ns(end));
                    }
                    return self.con_on_receive(bus, &f, end);
                }
                if now_ns >= to_ns + RX_DECISION_LAG_NS {
                    // Nothing came: the event ends here (a slave that missed
                    // its anchor, or a master whose slave did not answer).
                    return None;
                }
                let next_poll = (now_ns + POLL_NS).min(to_ns + RX_DECISION_LAG_NS);
                Some(self.elapsed_for_ns(next_poll).max(elapsed + 1))
            }
            LinkStep::ScanRx { from_ns } => {
                let end_ns = self.air_ns_at(ctx.event_end);
                let frames = self.air.frames_between(
                    &[ctx.channel],
                    from_ns,
                    now_ns.min(end_ns),
                    self.node_id,
                );
                for f in frames {
                    if f.access_address != ctx.access_address {
                        continue;
                    }
                    let fend = f.end_ns().unwrap_or(now_ns);
                    if fend > now_ns {
                        // Not finished yet: come back when it is, and do not
                        // skip past it.
                        return Some(self.elapsed_for_ns(fend));
                    }
                    self.set_link_step(LinkStep::ScanRx {
                        from_ns: f.air_ns.unwrap_or(from_ns) + 1,
                    });
                    if let Some(next) = self.scan_on_adv(bus, &ctx, &f, fend) {
                        return Some(next);
                    }
                }
                if elapsed >= ctx.event_end {
                    return None;
                }
                let next = self.elapsed_for_ns(now_ns + POLL_NS).min(ctx.event_end);
                Some(next.max(elapsed + 1))
            }
            LinkStep::ScanRspRx { at_ns } => {
                let f = self.air.receive_window(
                    ctx.channel,
                    ctx.access_address,
                    at_ns - IFS_TOLERANCE_NS,
                    at_ns + IFS_TOLERANCE_NS,
                    self.node_id,
                );
                if let Some(f) = f {
                    let fend = f.end_ns().unwrap_or(now_ns);
                    if fend > now_ns {
                        return Some(self.elapsed_for_ns(fend));
                    }
                    if f.pdu.first().map(|h| h & 0x0F) == Some(PDU_SCAN_RSP)
                        && self.write_adv_rx(bus, &ctx, &f)
                    {
                        self.raise_irq_bits(INT_SCH_PROG_RX);
                    }
                    self.set_link_step(LinkStep::ScanRx { from_ns: fend });
                } else if now_ns < at_ns + IFS_TOLERANCE_NS + RX_DECISION_LAG_NS {
                    return Some(
                        self.elapsed_for_ns(at_ns + IFS_TOLERANCE_NS + RX_DECISION_LAG_NS),
                    );
                } else {
                    self.set_link_step(LinkStep::ScanRx { from_ns: now_ns });
                }
                if elapsed >= ctx.event_end {
                    return None;
                }
                Some(elapsed + 1)
            }
        }
    }

    fn set_link_step(&mut self, step: LinkStep) {
        if let Some(l) = self.link.as_mut() {
            l.step = step;
        }
    }

    // ── Advertising ─────────────────────────────────────────────────────────

    /// A `SCAN_REQ` or `CONNECT_IND` started at T_IFS after our `ADV_IND`.
    /// Returns the next deadline if the event continues on the timed path.
    fn adv_on_request(&mut self, bus: &mut dyn Bus, ctx: &LinkCtx, f: &BleAirFrame) -> Option<u64> {
        if f.pdu.len() < 2 + 12 {
            return None;
        }
        let pdu_type = f.pdu[0] & 0x0F;
        let own = self.cs_bdaddr(bus, ctx.cs);
        // Both requests carry the advertiser's address at payload[6..12].
        if f.pdu[2 + 6..2 + 12] != own {
            return None;
        }
        let end = f.end_ns()?;
        match pdu_type {
            PDU_CONNECT_IND if f.pdu.len() == 2 + 34 => {
                if self.write_adv_rx(bus, ctx, f) {
                    self.raise_irq_bits(INT_SCH_PROG_RX);
                }
                if bt_trace_enabled() {
                    let nid = self.node_id;
                    eprintln!("[bt{nid}] adv: CONNECT_IND received — advertising ends");
                }
                self.set_link_step(LinkStep::EndAt);
                Some(self.elapsed_for_ns(end))
            }
            PDU_SCAN_REQ => {
                if self.write_adv_rx(bus, ctx, f) {
                    self.raise_irq_bits(INT_SCH_PROG_RX);
                }
                // The SCAN_RSP is the descriptor the ADV_IND's links to.
                let first = u32::from(self.em_read_u16(bus, ctx.cs + CS_TX_DESC_PTR)?);
                let next = u32::from(self.em_read_u16(bus, first + TXD_CNTL)? & 0x7FFF);
                if next != 0 {
                    if let Some(pdu) = self.adv_desc_pdu(bus, ctx.cs, next) {
                        if pdu[0] & 0x0F == PDU_SCAN_RSP {
                            self.air_tx(ctx, pdu, end + T_IFS_NS);
                        }
                    }
                }
                self.set_link_step(LinkStep::EndAt);
                Some(ctx.event_end.max(self.elapsed_for_ns(end)))
            }
            _ => None,
        }
    }

    /// The advertising PDU a TX descriptor describes: the header word, the
    /// device address from the control structure, then the descriptor's
    /// buffer — the same assembly [`Self::transmit_event`] measured.
    fn adv_desc_pdu(&self, bus: &mut dyn Bus, cs: u32, desc: u32) -> Option<Vec<u8>> {
        let hdr_word = self.em_read_u16(bus, desc + TXD_HEADER)?;
        let len = hdr_word >> 8;
        if len < TXD_ADDR_PREFIX_LEN {
            return None;
        }
        let data_ptr = u32::from(self.em_read_u16(bus, desc + TXD_DATA_PTR)?);
        let mut pdu = vec![(hdr_word & 0xFF) as u8, len as u8];
        for b in 0..u32::from(TXD_ADDR_PREFIX_LEN) {
            pdu.push(self.em_read_u8(bus, cs + CS_BDADDR + b).unwrap_or(0));
        }
        for b in 0..u32::from(len - TXD_ADDR_PREFIX_LEN) {
            pdu.push(self.em_read_u8(bus, data_ptr + b).unwrap_or(0));
        }
        Some(pdu)
    }

    fn cs_bdaddr(&self, bus: &mut dyn Bus, cs: u32) -> [u8; 6] {
        let mut a = [0u8; 6];
        for (i, b) in a.iter_mut().enumerate() {
            *b = self.em_read_u8(bus, cs + CS_BDADDR + i as u32).unwrap_or(0);
        }
        a
    }

    // ── Scanning / initiating ───────────────────────────────────────────────

    /// An advertising PDU finished arriving during an active scan or an
    /// initiator event.
    fn scan_on_adv(
        &mut self,
        bus: &mut dyn Bus,
        ctx: &LinkCtx,
        f: &BleAirFrame,
        fend: u64,
    ) -> Option<u64> {
        let pdu_type = *f.pdu.first()? & 0x0F;
        if f.pdu.len() < 2 + 6 {
            return None;
        }
        let adva: [u8; 6] = f.pdu[2..8].try_into().ok()?;
        match ctx.format {
            FMT_ACTIVE_SCAN => {
                if !matches!(
                    pdu_type,
                    PDU_ADV_IND | PDU_ADV_SCAN_IND | PDU_ADV_DIRECT_IND | 0x2
                ) {
                    return None;
                }
                if self.write_adv_rx(bus, ctx, f) {
                    self.raise_irq_bits(INT_SCH_PROG_RX);
                }
                if matches!(pdu_type, PDU_ADV_IND | PDU_ADV_SCAN_IND) {
                    // SCAN_REQ: ScanA (ours) + AdvA. TxAdd from our own address
                    // type is not tracked; the air does not filter on it.
                    let mut pdu = vec![PDU_SCAN_REQ | ((f.pdu[0] & 0x40) << 1), 12];
                    pdu.extend_from_slice(&self.cs_bdaddr(bus, ctx.cs));
                    pdu.extend_from_slice(&adva);
                    let req_at = fend + T_IFS_NS;
                    let req_end = req_at + airtime_ns(pdu.len());
                    self.air_tx(ctx, pdu, req_at);
                    let rsp_at = req_end + T_IFS_NS;
                    self.set_link_step(LinkStep::ScanRspRx { at_ns: rsp_at });
                    return Some(self.elapsed_for_ns(rsp_at + 80_000));
                }
                None
            }
            FMT_INITIATOR => {
                if !matches!(pdu_type, PDU_ADV_IND | PDU_ADV_DIRECT_IND) {
                    return None;
                }
                let mut target = [0u8; 6];
                for (i, b) in target.iter_mut().enumerate() {
                    *b = self
                        .em_read_u8(bus, ctx.cs + CS_ADV_BD_ADDR + i as u32)
                        .unwrap_or(0);
                }
                // Filter policy 0 (the only one Arduino/Bluedroid's
                // `connect(address)` uses): the target named in the CS. A
                // non-zero policy means the white list, which is not
                // modelled — accept any connectable advertiser then.
                let policy = self.em_read_u16(bus, ctx.cs + 0x14).unwrap_or(0) >> 8;
                if policy == 0 && adva != target {
                    return None;
                }
                self.initiate(bus, ctx, f, fend, adva)
            }
            _ => None,
        }
    }

    /// Send the `CONNECT_IND` the initiator's TX descriptor staged.
    fn initiate(
        &mut self,
        bus: &mut dyn Bus,
        ctx: &LinkCtx,
        adv: &BleAirFrame,
        adv_end: u64,
        adva: [u8; 6],
    ) -> Option<u64> {
        let desc = u32::from(self.em_read_u16(bus, ctx.cs + CS_TX_DESC_PTR)?);
        if desc == 0 {
            return None;
        }
        let hdr_word = self.em_read_u16(bus, desc + TXD_HEADER)?;
        let header = (hdr_word & 0xFF) as u8;
        if header & 0x0F != PDU_CONNECT_IND {
            return None;
        }
        let data_ptr = u32::from(self.em_read_u16(bus, desc + TXD_DATA_PTR)?);
        // LLData, 22 bytes, as software packed it (`lld_init_connect_req_pack`).
        let mut lldata = Vec::with_capacity(22);
        for b in 0..22 {
            lldata.push(self.em_read_u8(bus, data_ptr + b).unwrap_or(0));
        }
        let ci_at = adv_end + T_IFS_NS;
        let ci_len = 2 + 34;
        // WinOffset: software wrote the offset valid at the event start in
        // `CS+0x1E`; the core counts it down one 1.25 ms frame per frame
        // elapsed since, wrapping by the interval, and reports what it used
        // in `CS+0x44` ("left blank, filled by HW based on CS-WINOFFSET,
        // interval" — `lld_init_start`).
        let w0 = u64::from(self.em_read_u16(bus, ctx.cs + CS_WINOFFSET).unwrap_or(0));
        let intv = u64::from(self.em_read_u16(bus, ctx.cs + CS_CONNINTERVAL).unwrap_or(0)).max(1);
        let start_ns = self.air_ns_at(ctx.event_end.saturating_sub(self.radio_duration));
        let frames_elapsed = (ci_at.saturating_sub(start_ns)) / 1_250_000;
        let mut w = w0 as i64 - frames_elapsed as i64;
        while w < 0 {
            w += intv as i64;
        }
        let w = w as u16;
        lldata[8] = w as u8;
        lldata[9] = (w >> 8) as u8;
        let _ = self.em_write_u16(bus, ctx.cs + CS_TXWINOFFSET, w);

        let mut pdu = vec![header, 34];
        pdu.extend_from_slice(&self.cs_bdaddr(bus, ctx.cs));
        pdu.extend_from_slice(&adva);
        pdu.extend_from_slice(&lldata);
        // RxAdd follows the advertiser's TxAdd.
        pdu[0] = (pdu[0] & !0x80) | ((adv.pdu[0] & 0x40) << 1);
        self.air_tx(ctx, pdu, ci_at);

        if self.write_adv_rx(bus, ctx, adv) {
            self.raise_irq_bits(INT_SCH_PROG_RX);
        }
        let cntl = self.em_read_u16(bus, desc + TXD_CNTL).unwrap_or(0);
        let _ = self.em_write_u16(bus, desc + TXD_CNTL, cntl | TXD_DONE);
        if bt_trace_enabled() {
            let nid = self.node_id;
            eprintln!("[bt{nid}] init: CONNECT_IND sent to {adva:02x?}, winoffset {w}");
        }
        self.set_link_step(LinkStep::EndAt);
        Some(self.elapsed_for_ns(ci_at + airtime_ns(ci_len)))
    }

    // ── Connection events ───────────────────────────────────────────────────

    /// The data channel for this event: CSA #2 from the event counter when
    /// `HOP_SEL` is set, else CSA #1 (previous index + hop increment)
    /// remapped through the channel map.
    fn data_channel(&self, bus: &mut dyn Bus, cs: u32, hop: u16) -> u8 {
        let mut map = [false; 37];
        let m0 = self.em_read_u16(bus, cs + CS_CHMAP0).unwrap_or(0xFFFF);
        let m1 = self.em_read_u16(bus, cs + CS_CHMAP0 + 2).unwrap_or(0xFFFF);
        let m2 = self.em_read_u16(bus, cs + CS_CHMAP0 + 4).unwrap_or(0x1F) & 0x1F;
        let bits = u64::from(m0) | (u64::from(m1) << 16) | (u64::from(m2) << 32);
        for (i, used) in map.iter_mut().enumerate() {
            *used = bits & (1 << i) != 0;
        }
        if hop & HOP_SEL_CSA2 != 0 {
            let aa = self.em_read_u32(bus, cs + CS_ACCESS_ADDR).unwrap_or(0);
            let chan_id = ((aa >> 16) ^ (aa & 0xFFFF)) as u16;
            let counter = self.em_read_u16(bus, cs + CS_EVTCNT).unwrap_or(0);
            csa2_channel(counter, chan_id, &map)
        } else {
            csa1_remap(csa1_unmapped(hop), &map)
        }
    }

    /// Transmit the next connection packet at `at_ns`: the TX descriptor
    /// `CS+0x1C` names if firmware released it (`TXDONE` clear), else an empty
    /// PDU. `SN`/`NESN` come from `TXRXCNTL`.
    fn con_transmit(&mut self, bus: &mut dyn Bus, at_ns: u64) {
        let Some(ctx) = self.link.clone() else { return };
        let txrx = self.em_read_u16(bus, ctx.cs + CS_TXRXCNTL).unwrap_or(0);
        let sn = txrx & TXRX_SN != 0;
        let nesn = txrx & TXRX_NESN != 0;
        let desc = u32::from(self.em_read_u16(bus, ctx.cs + CS_TX_DESC_PTR).unwrap_or(0));
        let mut pdu;
        let ready = desc != 0
            && self
                .em_read_u16(bus, desc + TXD_CNTL)
                .is_some_and(|c| c & TXD_DONE == 0);
        if ready {
            let phce = self.em_read_u16(bus, desc + TXD_HEADER).unwrap_or(0);
            let len = (phce >> 8) as u8;
            let data_ptr = u32::from(self.em_read_u16(bus, desc + TXD_DATA_PTR).unwrap_or(0));
            let mut h = (phce as u8) & (PH_LLID | PH_MD);
            if sn {
                h |= PH_SN;
            }
            if nesn {
                h |= PH_NESN;
            }
            pdu = vec![h, len];
            for b in 0..u32::from(len) {
                pdu.push(self.em_read_u8(bus, data_ptr + b).unwrap_or(0));
            }
        } else {
            let mut h = 0x01; // LLID 01: empty / continuation, length 0
            if sn {
                h |= PH_SN;
            }
            if nesn {
                h |= PH_NESN;
            }
            pdu = vec![h, 0];
        }
        let empty = pdu[1] == 0;
        let txrx = (txrx & !TXRX_LASTEMPTY) | if empty { TXRX_LASTEMPTY } else { 0 };
        let _ = self.em_write_u16(bus, ctx.cs + CS_TXRXCNTL, txrx);
        let our_md = pdu[0] & PH_MD != 0;
        let end = at_ns + airtime_ns(pdu.len());
        if bt_trace_enabled() {
            let nid = self.node_id;
            let c = self.em_read_u16(bus, desc + TXD_CNTL).unwrap_or(0);
            eprintln!(
                "[bt{nid}] con TX ch{} {} desc={desc:#06x} cntl={c:#06x} pdu={:02x?}",
                ctx.channel,
                if ctx.format == FMT_MASTER { "M" } else { "S" },
                pdu
            );
        }
        self.air_tx(&ctx, pdu, at_ns);
        let packets = ctx.packets + 1;
        if let Some(l) = self.link.as_mut() {
            l.packets = packets;
        }
        // What comes next: a master always listens for the reply; a slave
        // listens for another master packet only if either side said MD.
        let expect_at = end + T_IFS_NS;
        let continue_rx = ctx.format == FMT_MASTER || our_md || ctx.peer_md;
        let event_end_ns = self.air_ns_at(ctx.event_end);
        if continue_rx && packets < MAX_PACKETS_PER_EVENT && expect_at < event_end_ns {
            self.set_link_step(LinkStep::ConRx {
                from_ns: expect_at - IFS_TOLERANCE_NS,
                to_ns: expect_at + IFS_TOLERANCE_NS,
            });
            let d = self.elapsed_for_ns(expect_at + 80_000);
            if let Some(l) = self.link.as_mut() {
                l.deadline = d;
            }
        } else {
            self.set_link_step(LinkStep::EndAt);
            let d = self.elapsed_for_ns(end);
            if let Some(l) = self.link.as_mut() {
                l.deadline = d;
            }
        }
    }

    /// A connection packet finished arriving. Update the acknowledgement
    /// state, hand it to firmware, and (slave) answer / (master) continue.
    fn con_on_receive(&mut self, bus: &mut dyn Bus, f: &BleAirFrame, end_ns: u64) -> Option<u64> {
        let ctx = self.link.clone()?;
        if f.pdu.len() < 2 {
            return None;
        }
        let h = f.pdu[0];
        let peer_sn = h & PH_SN != 0;
        let peer_nesn = h & PH_NESN != 0;
        let peer_md = h & PH_MD != 0;
        let mut txrx = self.em_read_u16(bus, ctx.cs + CS_TXRXCNTL).unwrap_or(0);
        let our_sn = txrx & TXRX_SN != 0;
        let our_nesn = txrx & TXRX_NESN != 0;

        // Acknowledgement of what we sent last: the peer's NESN moved past
        // our SN. On a slave's very first packet nothing is outstanding and
        // the master's NESN equals our SN (both 0).
        //
        // What was sent last lives in the control structure, not in this
        // event: a slave's reply is acknowledged by the master's first packet
        // of the NEXT event. `LASTEMPTY` clear means it came from the
        // descriptor `CS+0x1C` still names (the pointer only moves on an
        // acknowledgement), exactly the state the hardware keeps.
        let last_desc = if txrx & TXRX_LASTEMPTY == 0 {
            let d = u32::from(self.em_read_u16(bus, ctx.cs + CS_TX_DESC_PTR).unwrap_or(0));
            let pending = d != 0
                && self
                    .em_read_u16(bus, d + TXD_CNTL)
                    .is_some_and(|c| c & TXD_DONE == 0);
            pending.then_some(d)
        } else {
            None
        };
        if peer_nesn != our_sn {
            if let Some(desc) = last_desc {
                let cntl = self.em_read_u16(bus, desc + TXD_CNTL).unwrap_or(0);
                let _ = self.em_write_u16(bus, desc + TXD_CNTL, cntl | TXD_DONE);
                let next = u32::from(cntl & 0x7FFF);
                let _ = self.em_write_u16(bus, ctx.cs + CS_TX_DESC_PTR, next as u16);
                if bt_trace_enabled() {
                    let nid = self.node_id;
                    eprintln!("[bt{nid}] con ACK desc={desc:#06x} -> next={next:#06x}");
                }
                if let Some(l) = self.link.as_mut() {
                    l.tx_desc_acked += 1;
                }
                self.raise_irq_bits(INT_SCH_PROG_TX);
            }
            txrx ^= TXRX_SN;
        }

        // New data, or a retransmission of something already received.
        let fresh = peer_sn == our_nesn;
        let written = self.write_con_rx(bus, &ctx, f, fresh);
        if fresh && written {
            txrx ^= TXRX_NESN;
            txrx &= !TXRX_RXBUFF_FULL;
        } else if fresh && !written {
            // No free descriptor: NACK by leaving NESN, as the core does when
            // its RX buffers are full.
            txrx |= TXRX_RXBUFF_FULL;
        }
        let _ = self.em_write_u16(bus, ctx.cs + CS_TXRXCNTL, txrx);
        let used = self.link.as_ref().map(|l| l.rx_desc_used).unwrap_or(0);
        let acked = self.link.as_ref().map(|l| l.tx_desc_acked).unwrap_or(0);
        let _ = self.em_write_u16(
            bus,
            ctx.cs + CS_TXRXDESCCNT,
            ((used & 0xFF) << 8) | (acked & 0xFF),
        );
        if written {
            self.raise_irq_bits(INT_SCH_PROG_RX);
        }
        if let Some(l) = self.link.as_mut() {
            l.peer_md = peer_md;
        }
        let event_end_ns = self.air_ns_at(ctx.event_end);
        let reply_at = end_ns + T_IFS_NS;
        match ctx.format {
            FMT_SLAVE => {
                // A slave always answers the master's packet.
                self.con_transmit(bus, reply_at);
            }
            _ => {
                let our_more = self.tx_ready(bus, ctx.cs);
                if (peer_md || our_more)
                    && ctx.packets < MAX_PACKETS_PER_EVENT
                    && reply_at + 300_000 < event_end_ns
                {
                    self.con_transmit(bus, reply_at);
                } else {
                    return None;
                }
            }
        }
        self.link.as_ref().map(|l| l.deadline)
    }

    fn tx_ready(&self, bus: &mut dyn Bus, cs: u32) -> bool {
        let desc = u32::from(self.em_read_u16(bus, cs + CS_TX_DESC_PTR).unwrap_or(0));
        desc != 0
            && self
                .em_read_u16(bus, desc + TXD_CNTL)
                .is_some_and(|c| c & TXD_DONE == 0)
    }

    /// Write a received connection packet into the next RX descriptor.
    fn write_con_rx(
        &mut self,
        bus: &mut dyn Bus,
        ctx: &LinkCtx,
        f: &BleAirFrame,
        fresh: bool,
    ) -> bool {
        let status = if fresh {
            RXSTAT_CE_OK
        } else {
            RXSTAT_CE_SN_ERR
        };
        self.write_rx_desc(bus, ctx, f, status, true)
    }

    /// Write an advertising-channel PDU into the next RX descriptor.
    fn write_adv_rx(&mut self, bus: &mut dyn Bus, ctx: &LinkCtx, f: &BleAirFrame) -> bool {
        self.write_rx_desc(bus, ctx, f, RXSTAT_ADV_OK, false)
    }

    /// The RX-descriptor write both kinds share — the same ownership protocol
    /// as the scan path ([`RXD_DONE`] last, every core-owned field written),
    /// plus the sync timestamp and fine count `lld_con_rx` /
    /// `lld_adv_pkt_rx` turn into the new anchor.
    fn write_rx_desc(
        &mut self,
        bus: &mut dyn Bus,
        ctx: &LinkCtx,
        f: &BleAirFrame,
        status: u16,
        allow_null_buffer_if_empty: bool,
    ) -> bool {
        let rxd = self.reg(RX_DESC_PTR) & RX_DESC_PTR_MASK;
        if rxd < EM_RX_DESC_OFFSET || (rxd - EM_RX_DESC_OFFSET) % RX_DESC_BYTES != 0 {
            return false;
        }
        let Some(next_word) = self.em_read_u16(bus, rxd + RXD_NEXT) else {
            return false;
        };
        if next_word & RXD_DONE != 0 {
            if bt_trace_enabled() {
                let nid = self.node_id;
                eprintln!("[bt{nid}] link RX: rxd={rxd:#06x} still RXDONE — no free buffer");
            }
            return false;
        }
        let header = f.pdu[0];
        let payload = &f.pdu[2..];
        let data_ptr = self.em_read_u16(bus, rxd + RXD_DATA_PTR).unwrap_or(0);
        if data_ptr == 0 && !(allow_null_buffer_if_empty && payload.is_empty()) {
            return false;
        }
        for (i, b) in payload.iter().enumerate() {
            let Some(addr) = self.em_cpu_addr(u32::from(data_ptr) + i as u32) else {
                return false;
            };
            if crate::with_device_write_bypass(bus, |bus| bus.write_u8(addr, *b)).is_err() {
                return false;
            }
        }
        let sync_ns = f.air_ns.unwrap_or(0) + SYNC_OFFSET_NS;
        let (clkn, fine) = self.clkn_fine_of_ns(sync_ns);
        let _ = self.em_write_u16(
            bus,
            rxd + RXD_HEADER,
            ((payload.len() as u16) << 8) | u16::from(header),
        );
        let _ = self.em_write_u16(bus, rxd + RXD_STATUS, status);
        let _ = self.em_write_u16(bus, rxd + RXD_CHASS, u16::from(f.channel & 0x3F) << 8);
        let _ = self.em_write_u16(bus, rxd + RXD_TIMESTAMP, clkn as u16);
        let _ = self.em_write_u16(bus, rxd + RXD_TIMESTAMP + 2, ((clkn >> 16) & 0x0FFF) as u16);
        let _ = self.em_write_u16(
            bus,
            rxd + RXD_LINK_LABEL,
            (ctx.link_label << RXD_LINK_LABEL_SHIFT) | (fine & 0x3FF),
        );
        let _ = self.em_write_u16(bus, rxd + RXD_RAL_PTR, 0);
        let _ = self.em_write_u16(bus, rxd + RXD_UNKNOWN_10, 0);
        let _ = self.em_write_u16(bus, rxd + RXD_NEXT, next_word | RXD_DONE);
        let keep = self.reg(RX_DESC_PTR) & !RX_DESC_PTR_MASK;
        if let Some(slot) = self.regs.get_mut((RX_DESC_PTR / 4) as usize) {
            *slot = keep | (u32::from(next_word & RXD_NEXT_PTR_MASK) & RX_DESC_PTR_MASK);
        }
        if let Some(l) = self.link.as_mut() {
            l.rx_desc_used += 1;
        }
        if bt_trace_enabled() {
            let nid = self.node_id;
            eprintln!(
                "[bt{nid}] link RX ch{} rxd={rxd:#06x} label={} stat={status:#06x} pdu={:02x?}",
                f.channel, ctx.link_label, f.pdu
            );
        }
        true
    }

    /// Put a PDU on the air as this controller, stamped `at_ns`.
    fn air_tx(&self, ctx: &LinkCtx, pdu: Vec<u8>, at_ns: u64) {
        self.air.transmit(BleAirFrame {
            seq: 0,
            source: self.node_id,
            channel: ctx.channel,
            access_address: ctx.access_address,
            crc_init: ctx.crc_init,
            pdu,
            air_ns: Some(at_ns),
        });
    }
}

/// CSA #1's unmapped channel for the event a control structure describes.
///
/// With `FH_EN` set, `CH_IDX` holds the PREVIOUS event's unmapped channel and
/// the core adds `HOP_INT` itself. Measured: a slave whose first event is
/// `EVTCNT` 1 is programmed `HOPCNTL = 0x8707` (hop 7, index 7) and must
/// listen on 14 — the spec's (2 x hop) mod 37, where the central sends it —
/// and the next event is programmed index 14 and runs on 21.
pub fn csa1_unmapped(hop: u16) -> u8 {
    let idx = (hop & HOP_CH_IDX) as u8;
    if hop & HOP_FH_EN != 0 {
        (idx + ((hop >> 8) & 0x1F) as u8) % 37
    } else {
        idx
    }
}

/// Channel selection algorithm #1's remapping step (Core spec Vol 6 Part B
/// 4.5.8.2): an unused unmapped channel is replaced by the
/// `unmapped % numUsed`-th used channel.
pub fn csa1_remap(unmapped: u8, map: &[bool; 37]) -> u8 {
    let unmapped = unmapped % 37;
    if map[unmapped as usize] {
        return unmapped;
    }
    let used: Vec<u8> = (0..37u8).filter(|c| map[*c as usize]).collect();
    if used.is_empty() {
        return unmapped;
    }
    used[unmapped as usize % used.len()]
}

/// Channel selection algorithm #2 (Core spec Vol 6 Part B 4.5.8.3).
pub fn csa2_channel(counter: u16, chan_id: u16, map: &[bool; 37]) -> u8 {
    fn perm(x: u16) -> u16 {
        let lo = (x as u8).reverse_bits() as u16;
        let hi = ((x >> 8) as u8).reverse_bits() as u16;
        (hi << 8) | lo
    }
    fn mam(a: u16, b: u16) -> u16 {
        a.wrapping_mul(17).wrapping_add(b)
    }
    let mut x = counter ^ chan_id;
    for _ in 0..3 {
        x = perm(x);
        x = mam(x, chan_id);
    }
    let prn_e = x ^ chan_id;
    let unmapped = (prn_e % 37) as u8;
    if map[unmapped as usize] {
        return unmapped;
    }
    let used: Vec<u8> = (0..37u8).filter(|c| map[*c as usize]).collect();
    if used.is_empty() {
        return unmapped;
    }
    let idx = ((used.len() as u32 * u32::from(prn_e)) >> 16) as usize;
    used[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec's own CSA #2 sample data (Core spec Vol 6 Part C 3.1): access
    /// address 0x8E89BED6, all 37 channels used, counters 0..3 give 25, 20, 6,
    /// 21.
    #[test]
    fn csa2_matches_the_spec_sample_data() {
        let map = [true; 37];
        let aa: u32 = 0x8E89_BED6;
        let id = ((aa >> 16) ^ (aa & 0xFFFF)) as u16;
        assert_eq!(id, 0x305F);
        assert_eq!(csa2_channel(0, id, &map), 25);
        assert_eq!(csa2_channel(1, id, &map), 20);
        assert_eq!(csa2_channel(2, id, &map), 6);
        assert_eq!(csa2_channel(3, id, &map), 21);
    }

    /// CSA #2 sample data with 9 used channels (Vol 6 Part C 3.2): counters
    /// 6..8 give 23, 9, 34 (the last two through the remapping).
    #[test]
    fn csa2_remaps_through_a_partial_map() {
        let mut map = [false; 37];
        for c in [9, 10, 21, 22, 23, 33, 34, 35, 36] {
            map[c] = true;
        }
        let id = 0x305F;
        assert_eq!(csa2_channel(6, id, &map), 23);
        assert_eq!(csa2_channel(7, id, &map), 9);
        assert_eq!(csa2_channel(8, id, &map), 34);
    }

    /// `HOPCNTL = 0x8707` (FH_EN, hop 7, previous index 7) is event 1: 14.
    #[test]
    fn csa1_adds_the_hop_increment_when_the_core_hops() {
        assert_eq!(csa1_unmapped(0x8707), 14);
        assert_eq!(csa1_unmapped(0x870E), 21);
        assert_eq!(csa1_unmapped(0x8723), 5, "35 + 7 wraps mod 37");
        assert_eq!(csa1_unmapped(0x0707), 7, "FH_EN clear: CH_IDX as is");
    }

    #[test]
    fn csa1_remaps_unused_channels() {
        let mut map = [true; 37];
        map[5] = false;
        assert_eq!(csa1_remap(4, &map), 4);
        // 5 unused; 36 used channels; 5 % 36 = 5 -> the 6th used = channel 6.
        assert_eq!(csa1_remap(5, &map), 6);
    }
}
