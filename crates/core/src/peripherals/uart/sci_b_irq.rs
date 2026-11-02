// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Interrupt requests of the Renesas RA8 SCI_B in asynchronous mode.
//!
//! The SCI_B raises four interrupt EVENTS (FSP `bsp_elc.h`, RA8M1:
//! `ELC_EVENT_SCI9_RXI` 0x163, `_TXI` 0x164, `_TEI` 0x165, `_ERI` 0x166; every
//! channel's four are consecutive). None is wired to an NVIC line: the ICU
//! `IELSRn` links an event to line n and latches it in `IELSRn.IR`, so each
//! request is a single event (an edge), not a level. The handler clears `IR`
//! (`R_BSP_IrqStatusClear` / Zephyr's `IELSR_b[n].IR = 0`) and only the next
//! event re-pends the line.
//!
//! When the events fire (the CCR0 enables are RIE bit 16, TIE bit 20, TEIE
//! bit 21; FSP `R7FA8M1AH.h` `R_SCI_B0_CCR0_*`):
//! - **TXI**: when TDRE goes 1 with TIE = 1 — the byte in TDR has moved into
//!   the transmit shift register — and when TIE is set while TDRE is
//!   already 1. FSP's `r_sci_b_uart.c` sets TE and TIE together and Zephyr's
//!   `uart_ra_sci_b_irq_tx_enable` sets TIE with TE already on; both expect
//!   the TXI that follows.
//! - **TEI**: when TEND goes 1 with TEIE = 1 — the shift register has sent
//!   the stop bit and TDR is empty — and when TEIE is set while TEND is 1.
//! - **RXI**: once per character received with RIE = 1 and RE = 1, and no
//!   sooner than one frame after the previous RXI. The RX queue is filled
//!   all at once by an injection or a peer, but on a wire characters arrive
//!   a frame apart; without the spacing the next RXI would land while the
//!   handler for the last one still runs, before it clears `IELSRn.IR`,
//!   and be lost with it.
//!
//! The wire is timed so these edges come at the instants silicon gives them:
//! a byte written while the shift register is idle moves into it at once
//! (TDRE goes back to 1 at the write) and keeps it busy for one 10-bit frame;
//! a byte written while it is busy waits in TDR until the frame ends. TEND
//! goes 1 when the last frame ends. The frame length is 10 bits at `baud`,
//! counted in `clock_hz` cycles — the rate the guest believes its cycles run
//! at, the same convention as `ra_agt`. The register model is unchanged:
//! the byte still reaches the console at the write, and CSR still reads
//! TDRE = TEND = 1.
//!
//! NOT modelled: ERI (no receive errors are produced); the FIFO trigger
//! levels — with the FIFO (FRSR.R counts the queue) silicon raises RXI when
//! the count reaches FCR.RTRG, or 15 ETU after the last stop bit (FRSR.DR),
//! while this model raises it once per character, as for characters that
//! arrive one at a time; the baud rate actually programmed (CCR2.BRR/ABCS/
//! BGDM and the SCI clock are not decoded; `baud` comes from the chip YAML);
//! and an RXI for a character that arrived while RIE was 0 is raised when RIE
//! is set rather than lost.
//!
//! The classic SCI (RA6/RA4 SCIn, FSP `r_sci_uart`) raises the same four
//! events under the same rules; only its enables sit elsewhere, in the 8-bit
//! SCR (TIE bit 7, RIE 6, TE 5, RE 4, TEIE 2; FSP `R7FA6M5BH.h`
//! `R_SCI0_SCR_*_Pos`). [`classic_scr_as_ccr0`] moves them to the CCR0
//! positions, so one engine serves both. With the FIFO on, TXI follows TDFE
//! (FIFO count <= FCR.TTRG); FSP leaves TTRG at 0, so it comes when the last
//! queued byte enters the shift register — the TDRE edge above. RXI follows
//! RDF / DR (FCR.DRES = 0, which FSP leaves) and is raised per character, as
//! for SCI_B.

use std::sync::atomic::{AtomicU32, Ordering};

/// CCR0 bits (FSP `R_SCI_B0_CCR0_*_Pos`).
pub(super) const CCR0_RE: u32 = 1 << 0;
pub(super) const CCR0_TE: u32 = 1 << 4;
pub(super) const CCR0_RIE: u32 = 1 << 16;
pub(super) const CCR0_TIE: u32 = 1 << 20;
pub(super) const CCR0_TEIE: u32 = 1 << 21;

/// The classic SCI's SCR enables at their SCI_B CCR0 positions.
pub(super) fn classic_scr_as_ccr0(scr: u8) -> u32 {
    [
        (1 << 7, CCR0_TIE),
        (1 << 6, CCR0_RIE),
        (1 << 5, CCR0_TE),
        (1 << 4, CCR0_RE),
        (1 << 2, CCR0_TEIE),
    ]
    .into_iter()
    .filter(|&(bit, _)| scr & bit != 0)
    .fold(0, |ccr0, (_, flag)| ccr0 | flag)
}

/// Scheduler tokens for the TDRE / TEND edges: this base plus a 16-bit
/// generation. The shared UART's wake is 0.
pub(super) const SCI_B_EDGE_TOKEN: u32 = 0x5C1B_0000;
const GEN_MASK: u32 = 0xFFFF;

/// Bits per asynchronous frame (start, 8 data, stop).
const FRAME_BITS: u64 = 10;

#[derive(Debug, serde::Serialize)]
pub(super) struct SciBIrq {
    /// RXI event number; TXI, TEI and ERI follow it.
    rxi_event: u32,
    /// One frame in bus cycles; 0 means "no wire time" (edges at once).
    frame_cycles: u64,
    /// The cycle the shift register is free from.
    tsr_free_at: u64,
    /// Pending TDRE rising edge (the byte in TDR moves to the shift register).
    tdre_at: Option<u64>,
    /// Pending TEND rising edge.
    tend_at: Option<u64>,
    /// The character at the head of the RX queue already raised its RXI.
    rx_presented: bool,
    /// The earliest cycle the next RXI may be raised (one frame after the
    /// last).
    rx_next_at: u64,
    /// The deadline of the one live edge wake, if any. Only that wake
    /// (token `SCI_B_EDGE_TOKEN | generation`) is acted on; one armed earlier
    /// supersedes it, and the superseded wake is ignored when it fires. One
    /// wake per edge would pile up past the scheduler's per-peripheral
    /// ceiling when firmware writes TDR back to back.
    armed_at: Option<u64>,
    generation: u32,
    /// A wake to hand to the scheduler (`take_scheduled_events`).
    #[serde(skip)]
    new_wake: bool,
    /// RDR reads since the last service. `Peripheral::read` takes `&self`,
    /// so the pop is counted here and turned into "the next character is
    /// new" at the next service.
    #[serde(skip)]
    rx_pops: AtomicU32,
}

impl SciBIrq {
    pub(super) fn new(rxi_event: u32, clock_hz: u64, baud: u64) -> Self {
        let frame_cycles = (FRAME_BITS * clock_hz).checked_div(baud).unwrap_or(0);
        Self {
            rxi_event,
            frame_cycles,
            tsr_free_at: 0,
            tdre_at: None,
            tend_at: None,
            rx_presented: false,
            rx_next_at: 0,
            armed_at: None,
            generation: 0,
            new_wake: false,
            rx_pops: AtomicU32::new(0),
        }
    }

    pub(super) fn reset(&mut self) {
        // The generation survives, so a wake still queued from before the
        // reset is recognised as superseded.
        *self = Self {
            frame_cycles: self.frame_cycles,
            generation: self.generation.wrapping_add(1),
            ..Self::new(self.rxi_event, 0, 0)
        };
    }

    fn txi(&self) -> u32 {
        self.rxi_event + 1
    }

    fn tei(&self) -> u32 {
        self.rxi_event + 2
    }

    /// RXI needs a wakeup to notice a character that lands in the queue.
    pub(super) fn rx_armed(ccr0: u32) -> bool {
        ccr0 & (CCR0_RIE | CCR0_RE) == CCR0_RIE | CCR0_RE
    }

    pub(super) fn note_rx_pop(&self) {
        self.rx_pops.fetch_add(1, Ordering::Relaxed);
    }

    fn tdre(&self, now: u64) -> bool {
        self.tdre_at.is_none_or(|at| at <= now)
    }

    fn tend(&self, now: u64) -> bool {
        self.tsr_free_at <= now
    }

    /// A store to CCR0 moved it from `old` to `new`. A TIE (TEIE) set while
    /// TDRE (TEND) is already 1 raises TXI (TEI) at once.
    pub(super) fn on_ccr0(&mut self, old: u32, new: u32, now: u64) {
        let rising = new & !old;
        if rising & CCR0_TIE != 0 && self.tdre(now) {
            self.tdre_at = Some(now);
        }
        if rising & CCR0_TEIE != 0 && self.tend(now) {
            self.tend_at = Some(now);
        }
        self.arm();
    }

    /// A byte went into TDR at `now` (transmitter enabled).
    pub(super) fn on_tdr_write(&mut self, now: u64) {
        let start = self.tsr_free_at.max(now);
        // The byte enters the shift register when it is free; TDRE goes 1
        // then. Idle shifter: at the write itself.
        self.tdre_at = Some(start);
        self.tsr_free_at = start + self.frame_cycles;
        self.tend_at = Some(self.tsr_free_at);
        self.arm();
    }

    fn next_edge(&self) -> Option<u64> {
        [self.tdre_at, self.tend_at].into_iter().flatten().min()
    }

    fn token(&self) -> u32 {
        SCI_B_EDGE_TOKEN | (self.generation & GEN_MASK)
    }

    /// Arm a new wake when the earliest pending edge comes before the live
    /// one (or there is none).
    fn arm(&mut self) {
        let Some(next) = self.next_edge() else {
            return;
        };
        if self.armed_at.is_none_or(|armed| next < armed) {
            self.armed_at = Some(next);
            self.generation = self.generation.wrapping_add(1);
            self.new_wake = true;
        }
    }

    pub(super) fn is_edge_token(token: u32) -> bool {
        token & !GEN_MASK == SCI_B_EDGE_TOKEN
    }

    /// The scheduler entry `(delay, token)` for a wake armed since the last
    /// call, relative to `now`. The bus fires it at `now + 1 + delay`.
    pub(super) fn take_edge_wake(&mut self, now: u64) -> Option<(u64, u32)> {
        if !std::mem::take(&mut self.new_wake) {
            return None;
        }
        let at = self.armed_at?;
        Some((at.saturating_sub(now + 1), self.token()))
    }

    /// The live wake `token` fired at `now` (after `service`): the delay to
    /// re-arm it for the next pending edge, if any. A superseded wake gets
    /// nothing. An edge the clock has not reached yet is the next edge.
    pub(super) fn on_edge_wake(&mut self, token: u32, now: u64) -> Option<u64> {
        if token != self.token() || self.armed_at.is_none() {
            return None;
        }
        self.armed_at = self.next_edge();
        self.armed_at.map(|at| at.saturating_sub(now).max(1))
    }

    /// Raise the events due at `now`. `ccr0` is CCR0 as it is now, so an
    /// enable the handler dropped before the edge arrived raises nothing.
    pub(super) fn service(&mut self, now: u64, ccr0: u32, rx_waiting: bool) -> Vec<u32> {
        let mut events = Vec::new();
        let te = ccr0 & CCR0_TE != 0;
        if self.tdre_at.is_some_and(|at| at <= now) {
            self.tdre_at = None;
            if te && ccr0 & CCR0_TIE != 0 {
                events.push(self.txi());
            }
        }
        if self.tend_at.is_some_and(|at| at <= now) {
            self.tend_at = None;
            if te && ccr0 & CCR0_TEIE != 0 {
                events.push(self.tei());
            }
        }
        if self.rx_pops.swap(0, Ordering::Relaxed) != 0 || !rx_waiting {
            self.rx_presented = false;
        }
        if rx_waiting && !self.rx_presented && Self::rx_armed(ccr0) && now >= self.rx_next_at {
            self.rx_presented = true;
            self.rx_next_at = now + self.frame_cycles;
            events.push(self.rxi_event);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RXI: u32 = 0x163;
    const TXI: u32 = 0x164;
    const TEI: u32 = 0x165;
    const ON: u32 = CCR0_TE | CCR0_RE;

    /// 10 cycles per frame.
    fn sci() -> SciBIrq {
        SciBIrq::new(RXI, 1000, 1000)
    }

    #[test]
    fn frame_length_is_ten_bits_at_the_baud_in_guest_cycles() {
        assert_eq!(SciBIrq::new(RXI, 480_000_000, 115_200).frame_cycles, 41_666);
        assert_eq!(SciBIrq::new(RXI, 480_000_000, 0).frame_cycles, 0);
    }

    #[test]
    fn setting_tie_and_teie_on_an_idle_transmitter_raises_txi_and_tei() {
        let mut s = sci();
        s.on_ccr0(ON, ON | CCR0_TIE | CCR0_TEIE, 5);
        assert_eq!(
            s.service(5, ON | CCR0_TIE | CCR0_TEIE, false),
            vec![TXI, TEI]
        );
        // Edges, not levels: nothing more until something changes.
        assert!(s.service(6, ON | CCR0_TIE | CCR0_TEIE, false).is_empty());
    }

    #[test]
    fn tdr_writes_pace_txi_by_the_shift_register_and_tei_by_the_last_frame() {
        let mut s = sci();
        let ccr0 = ON | CCR0_TIE | CCR0_TEIE;
        // Idle shifter: the byte moves in at once, TDRE back at the write.
        s.on_tdr_write(100);
        let (delay, token) = s.take_edge_wake(99).expect("armed");
        assert_eq!(delay, 0);
        assert!(SciBIrq::is_edge_token(token));
        assert!(s.take_edge_wake(99).is_none());
        assert_eq!(s.service(100, ccr0, false), vec![TXI]);
        // Second byte while the first is on the wire: TDRE at its end.
        s.on_tdr_write(103);
        assert_eq!(s.next_edge(), Some(110));
        assert!(s.service(109, ccr0, false).is_empty());
        assert_eq!(s.service(110, ccr0, false), vec![TXI]);
        // TEND only after the second frame.
        assert!(s.service(119, ccr0, false).is_empty());
        assert_eq!(s.service(120, ccr0, false), vec![TEI]);
    }

    #[test]
    fn back_to_back_writes_keep_one_live_wake() {
        let mut s = sci();
        let ccr0 = ON | CCR0_TEIE;
        s.on_tdr_write(0);
        let (_, token) = s.take_edge_wake(0).expect("armed at the first TDRE");
        // Nine more bytes while the first is on the wire: no new wake.
        for t in 1..10 {
            s.on_tdr_write(t);
            assert!(s.take_edge_wake(t).is_none());
        }
        // The wake re-arms itself edge by edge; TEI only at the last frame.
        let mut now = 0;
        let mut tei_at = None;
        while let Some(delay) = {
            if !s.service(now, ccr0, false).is_empty() {
                tei_at = Some(now);
            }
            s.on_edge_wake(token, now)
        } {
            now += delay;
        }
        assert_eq!(tei_at, Some(100));
        // A superseded generation is ignored.
        s.on_ccr0(ON, ON | CCR0_TIE, 200);
        let (_, newer) = s.take_edge_wake(200).expect("TIE arms a wake");
        assert_ne!(newer, token);
        assert_eq!(s.on_edge_wake(token, 200), None);
    }

    #[test]
    fn an_enable_dropped_before_the_edge_raises_nothing() {
        let mut s = sci();
        s.on_tdr_write(0);
        // FSP's TXI handler writes the last byte, then swaps TIE for TEIE.
        s.service(0, ON | CCR0_TIE, false);
        s.on_tdr_write(1);
        s.on_ccr0(ON | CCR0_TIE, ON | CCR0_TEIE, 2);
        assert!(s.service(10, ON | CCR0_TEIE, false).is_empty());
        assert_eq!(s.service(20, ON | CCR0_TEIE, false), vec![TEI]);
        // TE off: nothing at all.
        let mut s = sci();
        s.on_ccr0(0, CCR0_TIE, 0);
        assert!(s.service(0, CCR0_TIE, false).is_empty());
    }

    #[test]
    fn teie_set_mid_frame_waits_for_tend() {
        let mut s = sci();
        s.on_tdr_write(0);
        s.service(0, ON, false);
        s.on_ccr0(ON, ON | CCR0_TEIE, 4);
        assert!(s.service(4, ON | CCR0_TEIE, false).is_empty());
        assert_eq!(s.service(10, ON | CCR0_TEIE, false), vec![TEI]);
    }

    #[test]
    fn rxi_is_raised_once_per_character_a_frame_apart() {
        let mut s = sci();
        let ccr0 = ON | CCR0_RIE;
        assert!(s.service(0, ccr0, false).is_empty());
        assert_eq!(s.service(1, ccr0, true), vec![RXI]);
        assert!(s.service(2, ccr0, true).is_empty());
        // RDR read with another character behind it: that one is new, and
        // it arrives one frame (10 cycles) after the last.
        s.note_rx_pop();
        assert!(s.service(3, ccr0, true).is_empty());
        assert!(s.service(10, ccr0, true).is_empty());
        assert_eq!(s.service(11, ccr0, true), vec![RXI]);
        // Last one read: queue empty, nothing.
        s.note_rx_pop();
        assert!(s.service(30, ccr0, false).is_empty());
        assert_eq!(s.service(31, ccr0, true), vec![RXI]);
    }

    #[test]
    fn classic_scr_enables_land_on_the_ccr0_positions() {
        assert_eq!(classic_scr_as_ccr0(0), 0);
        // FSP r_sci_uart: TE | RE | RIE, then Zephyr's irq_tx_enable adds
        // TIE | TEIE.
        assert_eq!(classic_scr_as_ccr0(0x70), CCR0_TE | CCR0_RE | CCR0_RIE);
        assert_eq!(
            classic_scr_as_ccr0(0xF4),
            CCR0_TIE | CCR0_RIE | CCR0_TE | CCR0_RE | CCR0_TEIE
        );
        // CKE (bits 1:0) and MPIE (bit 3) carry no interrupt enable.
        assert_eq!(classic_scr_as_ccr0(0x0B), 0);
    }

    #[test]
    fn rxi_needs_rie_and_re() {
        let mut s = sci();
        assert!(s.service(0, CCR0_RIE, true).is_empty());
        assert!(s.service(1, ON, true).is_empty());
        assert!(!SciBIrq::rx_armed(CCR0_RIE));
        // Enabled later: the waiting character raises its RXI then.
        assert_eq!(s.service(2, ON | CCR0_RIE, true), vec![RXI]);
    }
}
