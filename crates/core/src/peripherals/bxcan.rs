// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! bxCAN — STM32 basic-extended CAN controller (F1/F4/L4, RM0008 §24).
//!
//! Models the master control / status handshake (MCR.INRQ -> MSR.INAK,
//! MCR.SLEEP -> MSR.SLAK), the three TX mailboxes, RX FIFO0 with a working
//! **loopback** datapath (BTR.LBKM), an attachable `CanBus` interconnect,
//! **acceptance filtering** (FMR/FM1R/FS1R/FFA1R/FA1R + filter banks), and
//! **bit-timing validity** gating on TX.
//!
//! This model is deliberately *strict and silicon-accurate*: firmware that
//! would fail on a real STM32F103 fails here too (no false passes).
//!
//! - A received frame is delivered to a FIFO **only if an active filter
//!   matches**; otherwise it is dropped (RF0R.FMP0 stays 0). This mirrors
//!   the real chip, where a controller with no configured/active filter
//!   accepts nothing.
//! - A degenerate bit-timing (BTR with TS1 or TS2 segment = 0) makes every
//!   transmission fail with a bit error: the chip goes bus-off (ESR =
//!   0x00F8_0057), no RQCP/TXOK is raised and nothing is delivered. A valid
//!   BTR has TS1 >= 1 AND TS2 >= 1.
//!
//! Register reset values are silicon-pinned over SWD on an actual
//! STM32F103 (IDCODE 0x2003_6410, RM0008 §24.9.2).
//!
//! ## Timed CAN bus mode
//!
//! On a world's timed bus ([`crate::network::timed_can`], `attach_timed`) a
//! transmission takes wire time and can fail. A TXRQ leaves the mailbox
//! pending (TSR.TMEx clear) and stamps the request cycle; the world reads
//! the highest-priority pending mailbox (`timed_poll`) and later applies the
//! outcome (`timed_apply`), with RM0008 §24.9.2 semantics:
//!
//! - success: TXRQ clears, RQCPx | TXOKx; ALSTx / TERRx clear;
//! - lost arbitration: ALSTx, error: TERRx, TXOKx clear; the mailbox stays
//!   pending and is retried, unless MCR.NART, which releases it with RQCPx;
//! - ESR carries the bus's REC/TEC, LEC, BOFF, EPVF, EWGF (warning at 96);
//!   MSR.ERRI is set when an ESR flag is set whose IER enable is on;
//! - bus-off recovery (RM0008 §24.7.6) is counted by the bus once armed:
//!   at once with MCR.ABOM, otherwise when firmware leaves initialization;
//! - the four bxCAN interrupt lines are level sources: TX (RQCPx & TMEIE),
//!   RX0 (FMP0 & FMPIE0, FULL0 & FFIE0), RX1 (never: FIFO1 is not queued),
//!   SCE (ERRI & ERRIE).
//!
//! Not modelled in timed mode: TSR.LOWx, ABRQ of a frame already on the bus
//! (it aborts at the next attempt), silent mode (SILM: treated as off the
//! bus, it neither receives nor acknowledges), wake-up and sleep interrupts.

use crate::network::CanFrame;
use crate::peripherals::fdcan::FdcanTraceFrame;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};

// ---- Register offsets (RM0008 §24.9) ----
const REG_MCR: u64 = 0x000;
const REG_MSR: u64 = 0x004;
const REG_TSR: u64 = 0x008;
const REG_RF0R: u64 = 0x00C;
const REG_RF1R: u64 = 0x010;
const REG_IER: u64 = 0x014;
const REG_ESR: u64 = 0x018;
const REG_BTR: u64 = 0x01C;

/// First TX mailbox register (TI0R). Mailboxes are 0x10 apart: TIxR, TDTxR,
/// TDLxR, TDHxR.
const TX_MB_BASE: u64 = 0x180;
const TX_MB_END: u64 = 0x1B0;
/// RX FIFO0 mailbox: RI0R, RDT0R, RDL0R, RDH0R.
const RX_FIFO0_BASE: u64 = 0x1B0;
const RX_FIFO0_END: u64 = 0x1C0;

// ---- Filter registers (RM0008 §24.9.4, CAN_F* live in the same window) ----
const REG_FMR: u64 = 0x200; // filter master (FINIT bit0)
const REG_FM1R: u64 = 0x204; // filter mode: 0 = mask, 1 = list (per bank)
const REG_FS1R: u64 = 0x20C; // filter scale: 0 = 16-bit, 1 = 32-bit (per bank)
const REG_FFA1R: u64 = 0x214; // FIFO assignment: 0 = FIFO0, 1 = FIFO1 (per bank)
const REG_FA1R: u64 = 0x21C; // filter activation: 1 = active (per bank)
/// First filter bank register (F0R1). Each bank is two words (FxR1/FxR2),
/// banks are 8 bytes apart: 0x240 + n*8.
const FILTER_BANK_BASE: u64 = 0x240;
const FILTER_BANK_COUNT: u64 = 14;
const FILTER_BANK_END: u64 = FILTER_BANK_BASE + FILTER_BANK_COUNT * 8;

/// FMR reset on this single-CAN F103: FINIT=1 (bit0) plus the read-only
/// CAN2SB / reserved field 0x2A1C0E forced in the upper bits. Captured over
/// SWD. The whole word is preserved on readback; only FINIT is firmware-
/// observable.
const FMR_RESET: u32 = 0x2A1C_0E01;
const FMR_FINIT: u32 = 1 << 0;
/// Reserved/read-only bits forced into FMR on this part (everything but
/// FINIT — the CAN2SB field and reserved upper bits).
const FMR_FORCED_MASK: u32 = 0x2A1C_0E00;

const MCR_INRQ: u32 = 1 << 0;
const MCR_SLEEP: u32 = 1 << 1;
/// Transmit FIFO priority: mailboxes go out in request order, not by
/// identifier (RM0008 §24.7.1).
const MCR_TXFP: u32 = 1 << 2;
/// No automatic retransmission.
const MCR_NART: u32 = 1 << 4;
/// Automatic bus-off management.
const MCR_ABOM: u32 = 1 << 6;
const MCR_RESET: u32 = 1 << 15;

const MSR_ERRI: u32 = 1 << 2;

const TSR_RQCP: u32 = 1 << 0;
const TSR_TXOK: u32 = 1 << 1;
const TSR_ALST: u32 = 1 << 2;
const TSR_TERR: u32 = 1 << 3;
const TSR_ABRQ: u32 = 1 << 7;
const TSR_CODE_SHIFT: u32 = 24;

const IER_TMEIE: u32 = 1 << 0;
const IER_FMPIE0: u32 = 1 << 1;
const IER_FFIE0: u32 = 1 << 2;
const IER_EWGIE: u32 = 1 << 8;
const IER_EPVIE: u32 = 1 << 9;
const IER_BOFIE: u32 = 1 << 10;
const IER_LECIE: u32 = 1 << 11;
const IER_ERRIE: u32 = 1 << 15;

const ESR_EWGF: u32 = 1 << 0;
const ESR_EPVF: u32 = 1 << 1;
const ESR_BOFF: u32 = 1 << 2;

/// bxCAN1 interrupt vectors: USB_HP_CAN_TX, USB_LP_CAN_RX0, CAN_RX1,
/// CAN_SCE (RM0008 Table 63; the same positions on the F4, RM0090 Table 61,
/// and the L4, RM0351 Table 57). On the F103, TX and RX0 share their lines
/// with the USB device, which is not modelled.
const IRQ_TX: u32 = 19;
const IRQ_RX0: u32 = 20;
const IRQ_RX1: u32 = 21;
const IRQ_SCE: u32 = 22;

const TI_TXRQ: u32 = 1 << 0;
const TI_RTR: u32 = 1 << 1;
const TI_IDE: u32 = 1 << 2;

const RF_RFOM: u32 = 1 << 5; // release output mailbox (w1)

const BTR_LBKM: u32 = 1 << 30; // loopback mode
const BTR_SILM: u32 = 1 << 31; // silent mode
/// BTR bit 23 is a forced-0 reserved bit on this silicon (captured write
/// 0x40DC_0009 reads back 0x405C_0009). Mask it out of every BTR write.
const BTR_WRITE_MASK: u32 = 0xC37F_03FF & !(1 << 23);
/// TS1[3:0] occupy BTR bits 19:16; TS2[2:0] occupy bits 22:20.
const BTR_TS1_SHIFT: u32 = 16;
const BTR_TS2_SHIFT: u32 = 20;

/// Bus-off error state captured on the real chip when a degenerate BTR makes
/// every TX bit-error out: TEC = 0xF8 (bits 23:16), LEC = 0b101 bit-error
/// (bits 6:4), BOFF (bit2), EPVF (bit1), EWGF (bit0).
const ESR_BUS_OFF: u32 = 0x00F8_0057;

/// All three TX mailboxes empty: TME0/1/2 = bits 26,27,28.
const TSR_TME_ALL: u32 = 0x1C00_0000;
const FIFO0_DEPTH: usize = 3;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BxCan {
    mcr: u32,
    msr: u32,
    tsr: u32,
    rf1r: u32,
    ier: u32,
    esr: u32,
    btr: u32,
    /// Filter master register (FMR): FINIT + forced read-only field.
    fmr: u32,
    /// Filter mode (FM1R): 0 = mask, 1 = identifier-list, per bank bit.
    fm1r: u32,
    /// Filter scale (FS1R): 0 = dual 16-bit, 1 = single 32-bit, per bank bit.
    fs1r: u32,
    /// FIFO assignment (FFA1R): 0 = FIFO0, 1 = FIFO1, per bank bit.
    ffa1r: u32,
    /// Filter activation (FA1R): 1 = bank active, per bank bit.
    fa1r: u32,
    /// Filter bank registers F(n)R1/F(n)R2: 2 words per bank, 14 banks.
    filter_banks: [u32; (FILTER_BANK_COUNT * 2) as usize],
    /// Raw TX mailbox words, indexed by (offset - 0x180)/4 (12 words).
    tx_mb: [u32; 12],
    /// Filter banks and any other register-window storage (read-back).
    extra: HashMap<u64, u32>,
    /// RX FIFO0 contents (loopback / network delivery).
    #[serde(skip)]
    rx_fifo0: VecDeque<CanFrame>,
    /// Frames transmitted with loopback off and no bus attached. Bounded.
    #[serde(skip)]
    pub tx_frames: VecDeque<CanFrame>,
    /// The machine's ONE bus trace, and this controller's name in it. Private
    /// until `attach_bus_trace` hands over the shared handle at registration,
    /// so a bare `BxCan::new()` in a unit test still records somewhere.
    #[serde(skip)]
    trace: crate::bus::bus_trace::BusTrace,
    #[serde(skip)]
    trace_name: String,
    /// `CanBus` interconnect endpoints (`new_with_bus`). Transmitted frames go
    /// out on `bus_tx`; frames arriving on `bus_rx` are delivered (subject to
    /// the acceptance filter) on each tick while the controller is running.
    #[serde(skip)]
    bus_tx: Option<Sender<CanFrame>>,
    #[serde(skip)]
    bus_rx: Option<Receiver<CanFrame>>,
    /// The bus's cycle clock, which stamps TX requests and recovery arming.
    #[serde(skip)]
    clock: Option<crate::CycleClock>,
    /// On a world's timed CAN bus (see the module docs).
    #[serde(skip)]
    timed: bool,
    /// Timed mode: the cycle each mailbox's TXRQ was set at, and its place in
    /// request order (for MCR.TXFP).
    #[serde(skip)]
    tx_requested: [(u64, u64); 3],
    #[serde(skip)]
    next_request: u64,
    /// Timed mode: the bus has this controller bus-off.
    #[serde(skip)]
    bus_off: bool,
    /// Timed mode: bus-off recovery may count from this cycle.
    #[serde(skip)]
    recovery_armed: Option<u64>,
}

impl BxCan {
    pub fn new() -> Self {
        Self {
            // INRQ deasserted, SLEEP set per reset value (RM0008 §24.9.2).
            mcr: 0x0001_0002,
            // SLAK=1 (asleep) + SAMP=1 at reset (captured 0x0000_040A).
            msr: 0x0000_040A,
            tsr: TSR_TME_ALL,
            rf1r: 0,
            ier: 0,
            esr: 0,
            btr: 0x0123_0000,
            // FINIT=1 at reset + forced CAN2SB/reserved field (captured).
            fmr: FMR_RESET,
            fm1r: 0,
            fs1r: 0,
            ffa1r: 0,
            fa1r: 0,
            filter_banks: [0; (FILTER_BANK_COUNT * 2) as usize],
            tx_mb: [0; 12],
            extra: HashMap::new(),
            rx_fifo0: VecDeque::new(),
            tx_frames: VecDeque::new(),
            trace: crate::bus::bus_trace::BusTrace::new(),
            trace_name: String::new(),
            bus_tx: None,
            bus_rx: None,
            timed: false,
            clock: None,
            tx_requested: [(0, 0); 3],
            next_request: 0,
            bus_off: false,
            recovery_armed: None,
        }
    }

    /// Attach to a `CanBus` interconnect: transmitted frames go out on `tx`,
    /// frames arriving on `rx` are delivered (subject to acceptance filtering)
    /// on each tick while the controller is running.
    pub fn new_with_bus(tx: Sender<CanFrame>, rx: Receiver<CanFrame>) -> Self {
        let mut dev = Self::new();
        dev.bus_tx = Some(tx);
        dev.bus_rx = Some(rx);
        dev
    }

    pub fn attach_bus(
        &mut self,
        tx: Sender<CanFrame>,
        rx: Receiver<CanFrame>,
    ) -> anyhow::Result<()> {
        if self.bus_tx.is_some() || self.bus_rx.is_some() {
            anyhow::bail!("bxCAN is already attached to a CAN bus");
        }
        self.bus_tx = Some(tx);
        self.bus_rx = Some(rx);
        Ok(())
    }

    /// Put this controller on a world's timed CAN bus
    /// ([`crate::network::timed_can`]).
    pub fn attach_timed(&mut self) -> anyhow::Result<()> {
        if self.timed || self.bus_tx.is_some() || self.bus_rx.is_some() {
            anyhow::bail!("bxCAN is already attached to a CAN bus");
        }
        self.timed = true;
        Ok(())
    }

    /// `true` once on a timed CAN bus.
    pub fn is_timed(&self) -> bool {
        self.timed
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, |c| c.now())
    }

    fn mailbox_pending(&self, mb: usize) -> bool {
        self.tx_mb[mb * 4] & TI_TXRQ != 0
    }

    /// The pending mailbox that goes out next: lowest identifier first
    /// (lower mailbox number on a tie), or request order with MCR.TXFP
    /// (RM0008 §24.7.1).
    fn next_mailbox(&self) -> Option<usize> {
        let pending = (0..3).filter(|&mb| self.mailbox_pending(mb));
        if self.mcr & MCR_TXFP != 0 {
            pending.min_by_key(|&mb| self.tx_requested[mb].1)
        } else {
            pending.min_by_key(|&mb| {
                let frame = self
                    .decode_tx_mailbox(mb * 4)
                    .expect("mailbox index in range");
                (crate::network::timed_can::arbitration_key(&frame), mb)
            })
        }
    }

    /// The controller as the timed bus needs it; `pclk_hz` is its APB clock.
    ///
    /// Bit time (RM0008 §24.7.7): tq = (BRP + 1) / f_PCLK and one bit is
    /// 1 + (TS1 + 1) + (TS2 + 1) quanta.
    pub fn timed_poll(&self, pclk_hz: u64) -> crate::network::timed_can::CanNodePoll {
        use crate::network::timed_can::{CanNodePoll, CanTxRequest, ISO_RECOVERY_OCCURRENCES};
        let brp = u128::from(self.btr & 0x3FF) + 1;
        let ts1 = u128::from((self.btr >> BTR_TS1_SHIFT) & 0xF);
        let ts2 = u128::from((self.btr >> BTR_TS2_SHIFT) & 0x7);
        let bit_ps = (self.timing_valid() && pclk_hz > 0).then(|| {
            (brp * (3 + ts1 + ts2) * crate::network::timed_uart::PS_PER_SEC / u128::from(pclk_hz))
                as u64
        });
        let tx = self.next_mailbox().map(|mb| CanTxRequest {
            tag: mb as u32,
            frame: self
                .decode_tx_mailbox(mb * 4)
                .expect("mailbox index in range"),
            requested_cycle: self.tx_requested[mb].0,
        });
        CanNodePoll {
            bit_ps,
            clock_hz: pclk_hz,
            on_bus: self.running()
                && self.mcr & MCR_SLEEP == 0
                && self.btr & (BTR_LBKM | BTR_SILM) == 0,
            fd_capable: false,
            tx,
            recovery_armed_cycle: self.recovery_armed,
            recovery_occurrences: ISO_RECOVERY_OCCURRENCES,
        }
    }

    /// Apply what the timed bus decided for this controller.
    pub fn timed_apply(&mut self, update: &crate::network::timed_can::CanNodeUpdate) {
        use crate::network::timed_can::{CanErrorState, CanTxOutcome, ERROR_WARNING_LIMIT};
        // ESR (RM0008 §24.9.2): REC[31:24], TEC[23:16], LEC[6:4], BOFF,
        // EPVF, EWGF. LEC keeps its value unless the bus reports a new one.
        let old = self.esr;
        let lec = update.lec.map_or((old >> 4) & 0x7, u32::from);
        let warning = u16::from(update.tec) >= ERROR_WARNING_LIMIT
            || u16::from(update.rec) >= ERROR_WARNING_LIMIT;
        let mut flags = 0;
        if warning {
            flags |= ESR_EWGF;
        }
        if update.state != CanErrorState::Active {
            flags |= ESR_EPVF;
        }
        if update.state == CanErrorState::BusOff {
            flags |= ESR_BOFF;
        }
        self.esr =
            (u32::from(update.rec) << 24) | (u32::from(update.tec) << 16) | (lec << 4) | flags;
        // MSR.ERRI: an ESR error flag got set and its interrupt is enabled.
        let rising = flags & !old;
        let lec_error = update.lec.is_some_and(|l| l != 0);
        if (rising & ESR_EWGF != 0 && self.ier & IER_EWGIE != 0)
            || (rising & ESR_EPVF != 0 && self.ier & IER_EPVIE != 0)
            || (rising & ESR_BOFF != 0 && self.ier & IER_BOFIE != 0)
            || (lec_error && self.ier & IER_LECIE != 0)
        {
            self.msr |= MSR_ERRI;
        }
        // Bus-off entry and exit. With ABOM the recovery sequence starts at
        // once; without it firmware must enter and leave initialization
        // (RM0008 §24.7.6).
        let off = update.state == CanErrorState::BusOff;
        if off && !self.bus_off {
            self.recovery_armed = (self.mcr & MCR_ABOM != 0 && self.running()).then(|| self.now());
        } else if !off {
            self.recovery_armed = None;
        }
        self.bus_off = off;
        if let Some((tag, outcome)) = update.tx {
            let mb = tag as usize;
            if mb < 3 && self.mailbox_pending(mb) {
                let shift = mb as u32 * 8;
                self.tsr &= !((TSR_TXOK | TSR_ALST | TSR_TERR) << shift);
                let done = match outcome {
                    CanTxOutcome::Ok => {
                        self.tsr |= (TSR_RQCP | TSR_TXOK) << shift;
                        if let Some(frame) = self.decode_tx_mailbox(mb * 4) {
                            self.push_trace("tx", &frame);
                        }
                        true
                    }
                    CanTxOutcome::LostArbitration => {
                        self.tsr |= TSR_ALST << shift;
                        self.mcr & MCR_NART != 0
                    }
                    CanTxOutcome::Error | CanTxOutcome::NoAck => {
                        self.tsr |= TSR_TERR << shift;
                        self.mcr & MCR_NART != 0
                    }
                };
                if done {
                    self.tx_mb[mb * 4] &= !TI_TXRQ;
                    self.tsr |= TSR_RQCP << shift;
                }
            }
        }
        if let Some(frame) = &update.rx {
            let _ = self.try_deliver_rx(frame.clone());
        }
    }

    /// TSR as firmware reads it in timed mode: TMEx follows the mailbox's
    /// TXRQ, CODE names the next free mailbox, or with all three pending the
    /// one that goes out last (RM0008 §24.9.2).
    fn timed_tsr(&self) -> u32 {
        let mut tsr = self.tsr & !(TSR_TME_ALL | (0x3 << TSR_CODE_SHIFT));
        for mb in 0..3 {
            if !self.mailbox_pending(mb) {
                tsr |= 1 << (26 + mb);
            }
        }
        let code = match (0..3).find(|&mb| !self.mailbox_pending(mb)) {
            Some(free) => free as u32,
            None => {
                if self.mcr & MCR_TXFP != 0 {
                    (0..3)
                        .max_by_key(|&mb| self.tx_requested[mb].1)
                        .unwrap_or(0) as u32
                } else {
                    (0..3)
                        .max_by_key(|&mb| {
                            let frame = self.decode_tx_mailbox(mb * 4).expect("in range");
                            (crate::network::timed_can::arbitration_key(&frame), mb)
                        })
                        .unwrap_or(0) as u32
                }
            }
        };
        tsr | (code << TSR_CODE_SHIFT)
    }

    /// TSR writes in timed mode: RQCPx/TXOKx/ALSTx/TERRx are rc_w1, and
    /// clearing RQCPx clears the mailbox's other status bits with it;
    /// ABRQx aborts a pending request.
    fn timed_write_tsr(&mut self, value: u32) {
        for mb in 0..3u32 {
            let shift = mb * 8;
            let w = (value >> shift) & 0xFF;
            if w & TSR_RQCP != 0 {
                self.tsr &= !((TSR_RQCP | TSR_TXOK | TSR_ALST | TSR_TERR) << shift);
            } else {
                self.tsr &= !((w & (TSR_TXOK | TSR_ALST | TSR_TERR)) << shift);
            }
            if w & TSR_ABRQ != 0 && self.mailbox_pending(mb as usize) {
                self.tx_mb[mb as usize * 4] &= !TI_TXRQ;
                self.tsr &= !(TSR_TXOK << shift);
                self.tsr |= TSR_RQCP << shift;
            }
        }
    }

    /// Timed mode: TX (19), RX0 (20), RX1 (21) and SCE (22) as levels.
    fn timed_irq_levels(&self) -> [(u32, bool); 4] {
        let rqcp = TSR_RQCP | (TSR_RQCP << 8) | (TSR_RQCP << 16);
        let tx = self.ier & IER_TMEIE != 0 && self.tsr & rqcp != 0;
        let fill = self.rx_fifo0.len();
        let rx0 = (self.ier & IER_FMPIE0 != 0 && fill > 0)
            || (self.ier & IER_FFIE0 != 0 && fill >= FIFO0_DEPTH);
        let sce = self.ier & IER_ERRIE != 0 && self.msr & MSR_ERRI != 0;
        [
            (IRQ_TX, tx),
            (IRQ_RX0, rx0),
            (IRQ_RX1, false),
            (IRQ_SCE, sce),
        ]
    }

    /// Frame-level trace for the logic analyzer; mirrors the FDCAN shape so the
    /// shared CAN/UDS decoder works for both controllers.
    pub fn trace_snapshot(&self, peripheral: &str) -> Vec<FdcanTraceFrame> {
        crate::peripherals::can_trace_snapshot(&self.trace, &self.trace_name, peripheral)
    }

    fn push_trace(&mut self, direction: &'static str, frame: &CanFrame) {
        crate::peripherals::push_can_trace(&self.trace, &self.trace_name, direction, frame);
    }

    fn running(&self) -> bool {
        // Out of initialization (INRQ cleared) — frames may move.
        self.mcr & MCR_INRQ == 0
    }

    fn loopback(&self) -> bool {
        self.btr & BTR_LBKM != 0
    }

    /// Bit-timing validity: a real F103 needs nonzero TS1 and TS2 segments to
    /// sample a bit. A degenerate BTR (either segment 0) bit-errors every TX
    /// and the controller goes bus-off.
    fn timing_valid(&self) -> bool {
        let ts1 = (self.btr >> BTR_TS1_SHIFT) & 0xF;
        let ts2 = (self.btr >> BTR_TS2_SHIFT) & 0x7;
        ts1 >= 1 && ts2 >= 1
    }

    /// RF0R reflects the live FIFO0 fill level (FMP0[1:0]) and FULL0 (bit 3).
    fn rf0r(&self) -> u32 {
        let fmp = self.rx_fifo0.len().min(FIFO0_DEPTH) as u32;
        let full = if self.rx_fifo0.len() >= FIFO0_DEPTH {
            1 << 3
        } else {
            0
        };
        fmp | full
    }

    /// The 32-bit "ID register value" the silicon matches a frame against
    /// (RM0008 §24.7.4): standard = (id<<21)|(ide<<2)|(rtr<<1),
    /// extended = (id<<3)|(ide<<2)|(rtr<<1).
    fn frame_match_value(frame: &CanFrame) -> u32 {
        let ide = if frame.extended { 1 } else { 0 };
        let rtr = if frame.remote { 1 } else { 0 };
        let id = if frame.extended {
            (frame.id & 0x1FFF_FFFF) << 3
        } else {
            (frame.id & 0x7FF) << 21
        };
        id | (ide << 2) | (rtr << 1)
    }

    /// Does any **active** filter accept this frame? Returns the destination
    /// FIFO (0 or 1) when accepted, else None (frame dropped). Only an active
    /// filter (FA1R bit set) can match — with no active filter, nothing is
    /// accepted, exactly as on silicon.
    fn filter_accepts(&self, frame: &CanFrame) -> Option<u32> {
        let val = Self::frame_match_value(frame);
        for bank in 0..FILTER_BANK_COUNT as usize {
            let bank_bit = 1u32 << bank;
            if self.fa1r & bank_bit == 0 {
                continue; // bank inactive
            }
            let r1 = self.filter_banks[bank * 2];
            let r2 = self.filter_banks[bank * 2 + 1];
            let list_mode = self.fm1r & bank_bit != 0;
            let wide = self.fs1r & bank_bit != 0;
            let matched = if wide {
                // Single 32-bit filter.
                if list_mode {
                    val == r1 || val == r2
                } else {
                    (val & r2) == (r1 & r2)
                }
            } else {
                // Dual 16-bit: each 32-bit register holds two 16-bit
                // filters in [15:0] and [31:16] (RM0008 §24.7.4). The 16-bit
                // mapping packs STID[10:0], RTR, IDE, EXID[17:15] into the
                // top bits — match the high 16 bits of the frame value.
                let frame16 = (val >> 16) as u16;
                if list_mode {
                    // Four identifier-list filters: r1[15:0], r1[31:16],
                    // r2[15:0], r2[31:16].
                    let f0 = r1 as u16;
                    let f1 = (r1 >> 16) as u16;
                    let f2 = r2 as u16;
                    let f3 = (r2 >> 16) as u16;
                    frame16 == f0 || frame16 == f1 || frame16 == f2 || frame16 == f3
                } else {
                    // Two mask filters: (id=r1[15:0], mask=r1[31:16]) and
                    // (id=r2[15:0], mask=r2[31:16]).
                    let id_a = r1 as u16;
                    let mask_a = (r1 >> 16) as u16;
                    let id_b = r2 as u16;
                    let mask_b = (r2 >> 16) as u16;
                    (frame16 & mask_a) == (id_a & mask_a) || (frame16 & mask_b) == (id_b & mask_b)
                }
            };
            if matched {
                let fifo = if self.ffa1r & bank_bit != 0 { 1 } else { 0 };
                return Some(fifo);
            }
        }
        None
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            REG_MCR => self.mcr,
            REG_MSR => self.msr,
            REG_TSR if self.timed => self.timed_tsr(),
            REG_TSR => self.tsr,
            REG_RF0R => self.rf0r(),
            REG_RF1R => self.rf1r,
            REG_IER => self.ier,
            REG_ESR => self.esr,
            REG_BTR => self.btr,
            REG_FMR => self.fmr,
            REG_FM1R => self.fm1r,
            REG_FS1R => self.fs1r,
            REG_FFA1R => self.ffa1r,
            REG_FA1R => self.fa1r,
            o if (FILTER_BANK_BASE..FILTER_BANK_END).contains(&o) => {
                self.filter_banks[((o - FILTER_BANK_BASE) / 4) as usize]
            }
            o if (TX_MB_BASE..TX_MB_END).contains(&o) => {
                self.tx_mb[((o - TX_MB_BASE) / 4) as usize]
            }
            o if (RX_FIFO0_BASE..RX_FIFO0_END).contains(&o) => self.read_rx_fifo0(o),
            other => self.extra.get(&other).copied().unwrap_or(0),
        }
    }

    /// Present the front of RX FIFO0 through the RIxR/RDTxR/RDLxR/RDHxR window.
    fn read_rx_fifo0(&self, offset: u64) -> u32 {
        let Some(frame) = self.rx_fifo0.front() else {
            return 0;
        };
        match offset - RX_FIFO0_BASE {
            0x0 => {
                // RI0R: STID[31:21] or EXID[31:3], IDE, RTR.
                let id = if frame.extended {
                    (frame.id & 0x1FFF_FFFF) << 3 | TI_IDE
                } else {
                    (frame.id & 0x7FF) << 21
                };
                id | if frame.remote { TI_RTR } else { 0 }
            }
            0x4 => frame.data.len().min(8) as u32, // RDT0R: DLC[3:0], FMI=0, TIME=0
            0x8 => word_from_bytes(&frame.data, 0), // RDL0R: data[0..3]
            0xC => word_from_bytes(&frame.data, 4), // RDH0R: data[4..7]
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            REG_MCR => {
                let was_init = self.mcr & MCR_INRQ != 0;
                self.mcr = value & 0x0001_FF7F;
                if (self.mcr & MCR_INRQ) != 0 {
                    self.msr |= 1; // INAK
                    self.msr |= 1 << 3; // WKUI
                    self.msr &= !(1 << 1); // SLAK clears
                } else {
                    self.msr &= !1;
                }
                if (self.mcr & MCR_SLEEP) != 0 {
                    self.msr |= 1 << 1;
                } else {
                    self.msr &= !(1 << 1);
                }
                self.mcr &= !MCR_RESET; // RESET self-clears
                let in_init = self.mcr & MCR_INRQ != 0;
                if self.timed && self.bus_off && was_init != in_init {
                    // Without ABOM, recovery needs an INRQ cycle; in
                    // initialization the controller does not monitor the
                    // bus and cannot count (RM0008 §24.7.6).
                    self.recovery_armed = self.running().then(|| self.now());
                }
            }
            REG_MSR => self.msr &= !(value & 0x0000_001C), // rc_w1 flags
            REG_TSR if self.timed => self.timed_write_tsr(value),
            REG_TSR => {
                // rc_w1 for RQCP/TXOK/ALST/TERR; TME bits stay asserted.
                self.tsr &= !(value & 0x0F0F_0F0F);
                self.tsr |= TSR_TME_ALL;
            }
            REG_RF0R => {
                if value & RF_RFOM != 0 {
                    self.rx_fifo0.pop_front();
                }
            }
            REG_RF1R => self.rf1r = value & 0x37,
            REG_IER => self.ier = value & 0x0000_FFFF,
            REG_ESR => self.esr = (self.esr & !0x70) | (value & 0x70),
            REG_BTR => self.btr = value & BTR_WRITE_MASK,
            // FMR: only FINIT (bit0) is firmware-writable; the upper CAN2SB/
            // reserved field is forced read-only on this part.
            REG_FMR => self.fmr = (value & FMR_FINIT) | FMR_FORCED_MASK,
            REG_FM1R => self.fm1r = value & 0x3FFF,
            REG_FS1R => self.fs1r = value & 0x3FFF,
            REG_FFA1R => self.ffa1r = value & 0x3FFF,
            REG_FA1R => self.fa1r = value & 0x3FFF,
            o if (FILTER_BANK_BASE..FILTER_BANK_END).contains(&o) => {
                self.filter_banks[((o - FILTER_BANK_BASE) / 4) as usize] = value;
            }
            o if (TX_MB_BASE..TX_MB_END).contains(&o) => {
                let idx = ((o - TX_MB_BASE) / 4) as usize;
                self.tx_mb[idx] = value;
                // A write to TIxR with TXRQ set requests transmission.
                if (o - TX_MB_BASE) % 0x10 == 0 && value & TI_TXRQ != 0 {
                    self.request_tx((o - TX_MB_BASE) / 0x10);
                }
            }
            other => {
                self.extra.insert(other, value);
            }
        }
    }

    fn request_tx(&mut self, mailbox: u64) {
        if self.timed {
            // The bus decides when and whether it goes out; TXRQ stays set
            // and a new request clears the mailbox's status (RM0008 §24.9.2,
            // RQCPx "cleared by hardware on transmission request").
            let mb = mailbox as usize;
            self.tsr &= !((TSR_RQCP | TSR_TXOK | TSR_ALST | TSR_TERR) << (mb * 8));
            self.tx_requested[mb] = (self.now(), self.next_request);
            self.next_request += 1;
            return;
        }
        if !self.running() {
            return;
        }
        let base = (mailbox * 4) as usize; // 4 words per mailbox
        let Some(frame) = self.decode_tx_mailbox(base) else {
            return;
        };
        // Bit-timing gate: a degenerate BTR (TS1 or TS2 == 0) makes every TX
        // bit-error on real silicon. The controller goes bus-off; no RQCP/
        // TXOK, no delivery — TXRQ stays set (the request never completes).
        if !self.timing_valid() {
            self.esr = ESR_BUS_OFF;
            return;
        }
        // Clear TXRQ (mailbox emptied) and flag completion for this mailbox.
        self.tx_mb[base] &= !TI_TXRQ;
        let mb = mailbox as u32;
        self.tsr |= (1 << (mb * 8)) | (1 << (mb * 8 + 1)); // RQCPx | TXOKx
        self.tsr |= TSR_TME_ALL;

        self.push_trace("tx", &frame);
        if self.loopback() {
            self.deliver_rx(frame);
        } else if let Some(tx) = &self.bus_tx {
            let _ = tx.send(frame);
        } else {
            if self.tx_frames.len() >= 64 {
                self.tx_frames.pop_front();
            }
            self.tx_frames.push_back(frame);
        }
    }

    fn decode_tx_mailbox(&self, base: usize) -> Option<CanFrame> {
        let tir = *self.tx_mb.get(base)?;
        let tdtr = *self.tx_mb.get(base + 1)?;
        let tdlr = *self.tx_mb.get(base + 2)?;
        let tdhr = *self.tx_mb.get(base + 3)?;
        let extended = tir & TI_IDE != 0;
        let id = if extended {
            (tir >> 3) & 0x1FFF_FFFF
        } else {
            (tir >> 21) & 0x7FF
        };
        let len = (tdtr & 0xF).min(8) as usize;
        let mut data = Vec::with_capacity(len);
        for i in 0..len {
            let word = if i < 4 { tdlr } else { tdhr };
            data.push(((word >> ((i % 4) * 8)) & 0xFF) as u8);
        }
        Some(CanFrame {
            id,
            data,
            extended,
            fd: false,
            bitrate_switch: false,
            remote: tir & TI_RTR != 0,
        })
    }

    /// Deliver a frame into RX FIFO0 (loopback or external network), **subject
    /// to acceptance filtering**: a frame is accepted only if an active filter
    /// matches it; otherwise it is dropped (silicon-accurate — a controller
    /// with no active filter receives nothing). FIFO1 routing isn't modeled as
    /// a separate queue here, but a frame routed to FIFO1 by FFA1R is still
    /// not placed in FIFO0. Returns false when dropped or when the FIFO was
    /// full (FOVR0).
    pub fn deliver_rx(&mut self, frame: CanFrame) -> bool {
        self.try_deliver_rx(frame).is_ok()
    }

    /// [`Self::deliver_rx`], saying why a frame was not queued.
    pub fn try_deliver_rx(
        &mut self,
        frame: CanFrame,
    ) -> Result<(), crate::network::CanRxRejection> {
        use crate::network::CanRxRejection;
        if !self.running() {
            return Err(CanRxRejection::NotRunning);
        }
        // Acceptance filtering: drop the frame unless an active filter matches.
        let Some(fifo) = self.filter_accepts(&frame) else {
            return Err(CanRxRejection::NoFilterMatch);
        };
        // Only FIFO0 has a modeled queue; a FIFO1-routed frame is accepted by
        // a filter but not visible through the FIFO0 window.
        if fifo != 0 {
            self.push_trace("rx", &frame);
            return Err(CanRxRejection::Fifo1NotModeled);
        }
        if self.rx_fifo0.len() >= FIFO0_DEPTH {
            return Err(CanRxRejection::FifoFull);
        }
        self.push_trace("rx", &frame);
        self.rx_fifo0.push_back(frame);
        Ok(())
    }
}

fn word_from_bytes(data: &[u8], start: usize) -> u32 {
    let mut w = 0u32;
    for i in 0..4 {
        if let Some(b) = data.get(start + i) {
            w |= (*b as u32) << (i * 8);
        }
    }
    w
}

impl Default for BxCan {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for BxCan {
    fn bus_trace_handle(&self) -> Option<crate::bus::bus_trace::BusTrace> {
        Some(self.trace.clone())
    }

    /// Join the machine's one bus trace; see [`crate::bus::bus_trace`].
    fn attach_bus_trace(&mut self, name: &str, trace: &crate::bus::bus_trace::BusTrace) {
        self.trace = trace.clone();
        self.trace_name = name.to_string();
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = Peripheral::read_u32(self, offset & !3)?;
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) as u32 * 8;
        let current = Peripheral::read_u32(self, reg)?;
        let next = (current & !(0xFF << shift)) | ((value as u32) << shift);
        Peripheral::write_u32(self, reg, next)
    }

    // Word accessors are authoritative: RF0R.RFOM and TSR flags are w1
    // actions that must arrive atomically, not byte-decomposed (RMW would
    // re-trigger them on each lane — the F103 BSRR/SWIER failure class).
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }

    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        if self.timed {
            for (irq, level) in self.timed_irq_levels() {
                report(irq, level);
            }
        }
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.timed {
            // The world applies the bus's outcomes; the walk only holds the
            // interrupt lines at their levels.
            let irqs: Vec<u32> = self
                .timed_irq_levels()
                .into_iter()
                .filter(|(_, level)| *level)
                .map(|(irq, _)| irq)
                .collect();
            return PeripheralTickResult {
                explicit_irqs: (!irqs.is_empty()).then_some(irqs),
                ..Default::default()
            };
        }
        // Drain the interconnect into the receiver (subject to filtering).
        if let Some(rx) = self.bus_rx.take() {
            while let Ok(frame) = rx.try_recv() {
                self.deliver_rx(frame);
            }
            self.bus_rx = Some(rx);
        }
        PeripheralTickResult::with_irq(false)
    }

    fn needs_legacy_walk(&self) -> bool {
        // The ONLY thing bxCAN's `tick()` does is drain the `CanBus` mpsc
        // interconnect (`bus_rx`) into the receiver. With no interconnect
        // attached (`bus_rx == None`) the tick is a proven no-op for every
        // reachable firmware state, so the walk can be deleted byte-identically.
        //
        // Frame injection from a `can-player` external device does NOT flow
        // through this tick: it is pushed by `SystemBus::service_can_log_players`
        // (a per-cycle CAN orchestration service that runs whenever any player
        // is present — `per_cycle_tick_is_trivial` keeps the tick alive for it —
        // independent of walk deletion), so replay labs stay correct with the
        // walk deleted. The mpsc `bus_rx` path is used only by a multi-node
        // `CanBus` interconnect; when one is wired (`bus_rx == Some`) the walk
        // must stay on to poll it, and this reports `true` at that instant. The
        // interconnect is attached at construction (`new_with_bus`), before the
        // bus derives walk-deletion, so the derivation always sees the truth.
        //
        // On a timed bus the walk holds the four interrupt lines at their
        // levels, so it stays on there too.
        self.bus_rx.is_some() || self.timed
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(dev: &BxCan, offset: u64) -> u32 {
        Peripheral::read_u32(dev, offset).unwrap()
    }
    fn wr(dev: &mut BxCan, offset: u64, value: u32) {
        Peripheral::write_u32(dev, offset, value).unwrap()
    }

    /// A valid bit-timing (no loopback bit). The captured working loopback
    /// BTR was 0x40DC_0009 = LBKM | this timing word; readback 0x405C_0009
    /// (bit23 forced 0). TS1 = 0xC (>=1), TS2 = 0x5.
    const VALID_BTR: u32 = 0x00DC_0009;

    /// Install bank 0 as a 32-bit mask filter accepting exactly one standard
    /// ID into FIFO0: FS1R=1, FA1R=1, FM1R=0, F0R1=F0R2=(id<<21).
    fn install_std_accept_filter(dev: &mut BxCan, id: u32) {
        let val = (id & 0x7FF) << 21;
        wr(dev, REG_FS1R, 0x1); // bank0 32-bit
        wr(dev, REG_FM1R, 0x0); // bank0 mask mode
        wr(dev, REG_FFA1R, 0x0); // bank0 -> FIFO0
        wr(dev, FILTER_BANK_BASE, val); // F0R1 = id
        wr(dev, FILTER_BANK_BASE + 4, val); // F0R2 = mask (full)
        wr(dev, REG_FA1R, 0x1); // bank0 active
    }

    /// Leave init with loopback enabled and a valid bit-timing, mirroring
    /// HAL_CAN bring-up. (A real chip needs valid TS1/TS2 to transmit.)
    fn enter_loopback(dev: &mut BxCan) {
        wr(dev, REG_MCR, MCR_INRQ); // request init
        wr(dev, REG_BTR, VALID_BTR | BTR_LBKM); // valid timing + loopback
        wr(dev, REG_MCR, 0); // leave init -> running
    }

    /// Queue a standard-ID frame in TX mailbox 0 and request its transmission.
    fn send(dev: &mut BxCan, id: u32, data: &[u8]) {
        wr(dev, TX_MB_BASE + 0x08, word_from_bytes(data, 0)); // TDL0R
        wr(dev, TX_MB_BASE + 0x0C, word_from_bytes(data, 4)); // TDH0R
        wr(dev, TX_MB_BASE + 0x04, data.len() as u32); // TDT0R: DLC
        wr(dev, TX_MB_BASE, (id & 0x7FF) << 21 | TI_TXRQ); // TI0R
    }

    #[test]
    fn reset_values_match_silicon() {
        let dev = BxCan::new();
        assert_eq!(rd(&dev, REG_MCR), 0x0001_0002);
        assert_eq!(rd(&dev, REG_MSR), 0x0000_040A);
        assert_eq!(rd(&dev, REG_TSR), TSR_TME_ALL);
        assert_eq!(rd(&dev, REG_BTR), 0x0123_0000);
        assert_eq!(rd(&dev, REG_RF0R), 0);
    }

    #[test]
    fn reset_fmr_matches_silicon() {
        let dev = BxCan::new();
        // FINIT=1 + forced CAN2SB/reserved field on this single-CAN F103.
        assert_eq!(rd(&dev, REG_FMR), 0x2A1C_0E01);
    }

    #[test]
    fn fmr_forced_bits_preserved_on_write() {
        let mut dev = BxCan::new();
        // Firmware clears FINIT; the read-only upper field must survive.
        wr(&mut dev, REG_FMR, 0x0000_0000);
        assert_eq!(
            rd(&dev, REG_FMR),
            0x2A1C_0E00,
            "FINIT clears, forced bits stay"
        );
        wr(&mut dev, REG_FMR, 0xFFFF_FFFF);
        assert_eq!(rd(&dev, REG_FMR), 0x2A1C_0E01, "only FINIT settable");
    }

    #[test]
    fn btr_bit23_forced_zero() {
        let mut dev = BxCan::new();
        // Captured: write 0x40DC_0009, read back 0x405C_0009 (bit23 cleared).
        wr(&mut dev, REG_BTR, 0x40DC_0009);
        assert_eq!(rd(&dev, REG_BTR), 0x405C_0009);
    }

    #[test]
    fn inrq_init_handshake() {
        let mut dev = BxCan::new();
        wr(&mut dev, REG_MCR, MCR_INRQ);
        assert_ne!(rd(&dev, REG_MSR) & 1, 0, "INAK asserts");
        wr(&mut dev, REG_MCR, 0);
        assert_eq!(rd(&dev, REG_MSR) & 1, 0, "INAK clears");
    }

    #[test]
    fn loopback_frame_lands_in_rx_fifo0() {
        let mut dev = BxCan::new();
        enter_loopback(&mut dev);
        install_std_accept_filter(&mut dev, 0x111);
        send(
            &mut dev,
            0x111,
            &[0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33],
        );

        // One pending message; TX mailbox reported the send complete.
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 1, "FMP0 = 1");
        assert_ne!(rd(&dev, REG_TSR) & (1 << 1), 0, "TXOK0");
        // RI0R: standard ID 0x111 in [31:21], IDE clear.
        assert_eq!((rd(&dev, RX_FIFO0_BASE) >> 21) & 0x7FF, 0x111);
        assert_eq!(rd(&dev, RX_FIFO0_BASE) & TI_IDE, 0);
        // RDT0R: DLC 8.
        assert_eq!(rd(&dev, RX_FIFO0_BASE + 0x4) & 0xF, 8);
        // Payload byte-exact (RDL0R / RDH0R, little-endian within word).
        assert_eq!(rd(&dev, RX_FIFO0_BASE + 0x8), 0x01_27_0B_10);
        assert_eq!(rd(&dev, RX_FIFO0_BASE + 0xC), 0x33_22_11_5A);
    }

    #[test]
    fn captured_filter_accepts_111_rejects_222() {
        // Captured proof: bank0 32-bit mask, F0R1=F0R2=0x2220_0000 (=0x111<<21).
        // An 0x111 std frame is ACCEPTED; an 0x222 std frame is REJECTED.
        let mut dev = BxCan::new();
        enter_loopback(&mut dev);
        wr(&mut dev, REG_FS1R, 0x1);
        wr(&mut dev, REG_FM1R, 0x0);
        wr(&mut dev, REG_FFA1R, 0x0);
        wr(&mut dev, FILTER_BANK_BASE, 0x2220_0000); // 0x111 << 21
        wr(&mut dev, FILTER_BANK_BASE + 4, 0x2220_0000);
        wr(&mut dev, REG_FA1R, 0x1);

        send(&mut dev, 0x111, &[0xAA]);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 1, "0x111 accepted");
        // Drain, then send a non-matching ID.
        wr(&mut dev, REG_RF0R, RF_RFOM);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0);
        send(&mut dev, 0x222, &[0xBB]);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0, "0x222 rejected (FMP0 stays 0)");
    }

    #[test]
    fn no_filter_means_no_rx() {
        // Strictness: with no active filter, a real chip accepts nothing.
        let mut dev = BxCan::new();
        enter_loopback(&mut dev);
        send(&mut dev, 0x111, &[0x01, 0x02]);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0, "no active filter -> dropped");
        // TX still completed though (valid timing).
        assert_ne!(rd(&dev, REG_TSR) & (1 << 1), 0, "TXOK0 still set");
    }

    #[test]
    fn degenerate_btr_tx_fails_and_goes_bus_off() {
        let mut dev = BxCan::new();
        wr(&mut dev, REG_MCR, MCR_INRQ);
        wr(&mut dev, REG_BTR, BTR_LBKM); // TS1=TS2=0: degenerate
        wr(&mut dev, REG_MCR, 0);
        install_std_accept_filter(&mut dev, 0x111);
        send(&mut dev, 0x111, &[0x42]);

        // No completion, no delivery; ESR shows the captured bus-off state.
        assert_eq!(rd(&dev, REG_TSR) & 0x3, 0, "no RQCP/TXOK0");
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0, "FMP0 stays 0");
        assert_eq!(rd(&dev, REG_ESR), 0x00F8_0057, "captured bus-off ESR");
    }

    #[test]
    fn valid_btr_with_matching_filter_delivers() {
        let mut dev = BxCan::new();
        enter_loopback(&mut dev);
        install_std_accept_filter(&mut dev, 0x111);
        send(&mut dev, 0x111, &[0x10, 0x0B, 0x27, 0x01]);
        assert_eq!(
            rd(&dev, REG_RF0R) & 0x3,
            1,
            "delivered with valid timing + filter"
        );
        assert_eq!(rd(&dev, REG_ESR), 0, "no error state");
        assert_eq!((rd(&dev, RX_FIFO0_BASE) >> 21) & 0x7FF, 0x111);
    }

    #[test]
    fn rfom_release_advances_the_fifo() {
        let mut dev = BxCan::new();
        enter_loopback(&mut dev);
        // Accept both 0x111 and 0x222 via a mask that ignores the differing
        // bit: a list-mode bank with both IDs.
        wr(&mut dev, REG_FS1R, 0x1);
        wr(&mut dev, REG_FM1R, 0x1); // bank0 list mode
        wr(&mut dev, FILTER_BANK_BASE, (0x111 & 0x7FF) << 21); // F0R1 = 0x111
        wr(&mut dev, FILTER_BANK_BASE + 4, (0x222 & 0x7FF) << 21); // F0R2 = 0x222
        wr(&mut dev, REG_FA1R, 0x1);

        send(&mut dev, 0x111, &[0xAA]);
        send(&mut dev, 0x222, &[0xBB]);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 2);
        // Release the head: next frame (0x222) becomes visible.
        wr(&mut dev, REG_RF0R, RF_RFOM);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 1);
        assert_eq!((rd(&dev, RX_FIFO0_BASE) >> 21) & 0x7FF, 0x222);
        wr(&mut dev, REG_RF0R, RF_RFOM);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0, "FIFO drained");
        assert_eq!(rd(&dev, RX_FIFO0_BASE), 0, "empty FIFO reads 0");
    }

    #[test]
    fn no_loopback_when_still_in_init() {
        let mut dev = BxCan::new();
        wr(&mut dev, REG_MCR, MCR_INRQ); // stay in init
        wr(&mut dev, REG_BTR, VALID_BTR | BTR_LBKM);
        install_std_accept_filter(&mut dev, 0x111);
        send(&mut dev, 0x111, &[0x01, 0x02]);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0, "no delivery while INRQ set");
    }

    #[test]
    fn loopback_frames_are_exposed_as_can_trace_events() {
        let mut dev = BxCan::new();
        enter_loopback(&mut dev);
        install_std_accept_filter(&mut dev, 0x111);
        send(
            &mut dev,
            0x111,
            &[0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33],
        );
        let trace = dev.trace_snapshot("bxcan1");
        // One loopback frame appears as a tx then an rx event.
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].peripheral, "bxcan1");
        assert_eq!(trace[0].direction, "tx");
        assert_eq!(trace[0].id, 0x111);
        assert_eq!(trace[1].direction, "rx");
        assert_eq!(trace[1].id, 0x111);
        assert_eq!(
            trace[1].data,
            vec![0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33]
        );
    }

    #[test]
    fn non_loopback_tx_is_buffered_not_received() {
        let mut dev = BxCan::new();
        wr(&mut dev, REG_MCR, MCR_INRQ);
        wr(&mut dev, REG_BTR, VALID_BTR); // valid timing, no loopback
        wr(&mut dev, REG_MCR, 0); // running
        send(&mut dev, 0x111, &[0x42]);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0, "not looped back");
        assert_eq!(dev.tx_frames.len(), 1);
        assert_eq!(dev.tx_frames[0].id, 0x111);
    }

    #[test]
    fn bus_path_tx_sends_and_rx_filters() {
        use std::sync::mpsc::channel;
        let (out_tx, out_rx) = channel::<CanFrame>();
        let (in_tx, in_rx) = channel::<CanFrame>();
        let mut dev = BxCan::new_with_bus(out_tx, in_rx);
        wr(&mut dev, REG_MCR, MCR_INRQ);
        wr(&mut dev, REG_BTR, VALID_BTR); // valid timing, no loopback
        wr(&mut dev, REG_MCR, 0);
        install_std_accept_filter(&mut dev, 0x111);

        // TX goes out on the bus, not into tx_frames.
        send(&mut dev, 0x123, &[0x01, 0x02]);
        let sent = out_rx.try_recv().expect("frame on bus");
        assert_eq!(sent.id, 0x123);
        assert!(dev.tx_frames.is_empty());

        // RX from the bus: matching ID delivered, non-matching dropped.
        in_tx.send(CanFrame::classic(0x111, vec![0xAB])).unwrap();
        in_tx.send(CanFrame::classic(0x222, vec![0xCD])).unwrap();
        dev.tick();
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 1, "only 0x111 accepted");
        assert_eq!((rd(&dev, RX_FIFO0_BASE) >> 21) & 0x7FF, 0x111);
    }

    // ---- Timed CAN bus mode ----

    use crate::network::timed_can::{lec, CanErrorState, CanNodeUpdate, CanTxOutcome};

    /// BTR for 500 kbit/s from a 36 MHz APB1: BRP = 3 (tq = 4 / 36 MHz),
    /// TS1 = 13 (14 tq), TS2 = 2 (3 tq): 1 + 14 + 3 = 18 tq per bit.
    const BTR_500K_AT_36M: u32 = (2 << BTR_TS2_SHIFT) | (13 << BTR_TS1_SHIFT) | 3;
    /// The same with BRP = 7: 250 kbit/s, the field bug.
    const BTR_250K_AT_36M: u32 = (2 << BTR_TS2_SHIFT) | (13 << BTR_TS1_SHIFT) | 7;

    const IER_TMEIE: u32 = 1 << 0;
    const IER_FMPIE0: u32 = 1 << 1;
    const IER_EWGIE: u32 = 1 << 8;
    const IER_EPVIE: u32 = 1 << 9;
    const IER_BOFIE: u32 = 1 << 10;
    const IER_LECIE: u32 = 1 << 11;
    const IER_ERRIE: u32 = 1 << 15;
    const MSR_ERRI: u32 = 1 << 2;

    fn timed(btr: u32) -> BxCan {
        let mut dev = BxCan::new();
        dev.attach_timed().unwrap();
        dev.attach_cycle_clock(crate::CycleClock::default());
        wr(&mut dev, REG_MCR, MCR_INRQ);
        wr(&mut dev, REG_BTR, btr);
        wr(&mut dev, REG_MCR, 0);
        dev
    }

    fn send_mb(dev: &mut BxCan, mb: u64, id: u32, data: &[u8]) {
        let base = TX_MB_BASE + mb * 0x10;
        wr(dev, base + 0x08, word_from_bytes(data, 0));
        wr(dev, base + 0x0C, word_from_bytes(data, 4));
        wr(dev, base + 0x04, data.len() as u32);
        wr(dev, base, (id & 0x7FF) << 21 | TI_TXRQ);
    }

    fn update(
        tx: Option<(u32, CanTxOutcome)>,
        tec: u8,
        rec: u8,
        state: CanErrorState,
    ) -> CanNodeUpdate {
        CanNodeUpdate {
            tx,
            rx: None,
            tec,
            rec,
            state,
            lec: None,
        }
    }

    #[test]
    fn timed_bit_time_comes_from_btr_and_the_apb_clock() {
        // RM0008 §24.7.7: tq = (BRP+1) / f_PCLK, bit = tq x (1 + (TS1+1) + (TS2+1)).
        let dev = timed(BTR_500K_AT_36M);
        let poll = dev.timed_poll(36_000_000);
        assert_eq!(poll.bit_ps, Some(2_000_000), "500 kbit/s");
        assert_eq!(poll.clock_hz, 36_000_000);
        assert!(poll.on_bus);
        assert_eq!(
            timed(BTR_250K_AT_36M).timed_poll(36_000_000).bit_ps,
            Some(4_000_000)
        );
        // The silicon-pinned degenerate BTR (TS1 or TS2 = 0) has no bit time.
        let mut bad = timed(BTR_500K_AT_36M);
        wr(&mut bad, REG_MCR, MCR_INRQ);
        wr(&mut bad, REG_BTR, 3);
        assert!(!bad.timed_poll(36_000_000).on_bus, "in init: off the bus");
        wr(&mut bad, REG_MCR, 0);
        assert_eq!(bad.timed_poll(36_000_000).bit_ps, None);
        // Loopback keeps the controller off the shared bus.
        let mut lb = timed(BTR_500K_AT_36M | BTR_LBKM);
        assert!(!lb.timed_poll(36_000_000).on_bus);
        wr(&mut lb, REG_MCR, MCR_INRQ);
    }

    #[test]
    fn timed_tx_stays_pending_until_the_bus_reports_it() {
        let mut dev = timed(BTR_500K_AT_36M);
        dev.clock.as_ref().unwrap().publish(1234);
        send_mb(&mut dev, 0, 0x100, &[0x0F, 0xA0]);
        // RM0008 §24.9.2 TSR: TME0 clears while TXRQ0 is pending; no RQCP yet.
        assert_eq!(rd(&dev, REG_TSR) & (1 << 26), 0, "TME0 clear");
        assert_eq!(rd(&dev, REG_TSR) & 0x3, 0, "no RQCP0/TXOK0");
        assert_eq!(rd(&dev, TX_MB_BASE) & TI_TXRQ, TI_TXRQ);
        let tx = dev.timed_poll(36_000_000).tx.expect("pending request");
        assert_eq!((tx.tag, tx.frame.id, tx.requested_cycle), (0, 0x100, 1234));
        assert_eq!(tx.frame.data, vec![0x0F, 0xA0]);
        dev.timed_apply(&update(
            Some((0, CanTxOutcome::Ok)),
            0,
            0,
            CanErrorState::Active,
        ));
        assert_ne!(rd(&dev, REG_TSR) & (1 << 26), 0, "TME0 set again");
        assert_eq!(rd(&dev, REG_TSR) & 0x3, 0x3, "RQCP0 | TXOK0");
        assert_eq!(rd(&dev, TX_MB_BASE) & TI_TXRQ, 0);
        assert!(dev.timed_poll(36_000_000).tx.is_none());
        assert!(dev.tx_frames.is_empty(), "the bus carried it");
    }

    #[test]
    fn timed_failed_attempts_set_terr_or_alst_and_retry_unless_nart() {
        let mut dev = timed(BTR_500K_AT_36M);
        send_mb(&mut dev, 1, 0x200, &[1]);
        dev.timed_apply(&update(
            Some((1, CanTxOutcome::Error)),
            8,
            0,
            CanErrorState::Active,
        ));
        let tsr = rd(&dev, REG_TSR);
        assert_eq!((tsr >> 8) & 0xF, 0b1000, "TERR1, no RQCP1/TXOK1");
        assert!(
            dev.timed_poll(36_000_000).tx.is_some(),
            "auto-retransmission"
        );
        dev.timed_apply(&update(
            Some((1, CanTxOutcome::LostArbitration)),
            8,
            1,
            CanErrorState::Active,
        ));
        assert_eq!(
            (rd(&dev, REG_TSR) >> 8) & 0xF,
            0b0100,
            "ALST1 for the last attempt"
        );
        dev.timed_apply(&update(
            Some((1, CanTxOutcome::Ok)),
            7,
            1,
            CanErrorState::Active,
        ));
        assert_eq!((rd(&dev, REG_TSR) >> 8) & 0xF, 0b0011, "RQCP1 | TXOK1");

        // NART (MCR bit 4): one attempt, then the mailbox is released.
        let mut dev = timed(BTR_500K_AT_36M);
        wr(&mut dev, REG_MCR, 1 << 4);
        send_mb(&mut dev, 0, 0x200, &[1]);
        dev.timed_apply(&update(
            Some((0, CanTxOutcome::NoAck)),
            8,
            0,
            CanErrorState::Active,
        ));
        assert_eq!(
            rd(&dev, REG_TSR) & 0xF,
            0b1001,
            "RQCP0 | TERR0, TXOK0 clear"
        );
        assert!(dev.timed_poll(36_000_000).tx.is_none());
        assert_ne!(rd(&dev, REG_TSR) & (1 << 26), 0, "TME0");
    }

    #[test]
    fn timed_mailbox_priority_follows_the_identifier_or_txfp() {
        let mut dev = timed(BTR_500K_AT_36M);
        send_mb(&mut dev, 0, 0x300, &[]);
        send_mb(&mut dev, 1, 0x100, &[]);
        // RM0008 §24.7.1: by identifier when TXFP = 0.
        assert_eq!(dev.timed_poll(36_000_000).tx.unwrap().tag, 1);
        // TSR.CODE names the next free mailbox: 2.
        assert_eq!((rd(&dev, REG_TSR) >> 24) & 0x3, 2);
        let mut dev = timed(BTR_500K_AT_36M);
        wr(&mut dev, REG_MCR, 1 << 2); // TXFP: request order
        send_mb(&mut dev, 0, 0x300, &[]);
        send_mb(&mut dev, 1, 0x100, &[]);
        assert_eq!(dev.timed_poll(36_000_000).tx.unwrap().tag, 0);
    }

    #[test]
    fn timed_esr_mirrors_the_bus_counters_and_state() {
        let mut dev = timed(BTR_500K_AT_36M);
        let mut u = update(None, 136, 5, CanErrorState::Passive);
        u.lec = Some(lec::BIT_RECESSIVE);
        dev.timed_apply(&u);
        // RM0008 §24.9.2 ESR: REC[31:24], TEC[23:16], LEC[6:4], BOFF, EPVF, EWGF.
        assert_eq!(
            rd(&dev, REG_ESR),
            (5 << 24) | (136 << 16) | (4 << 4) | 0b011
        );
        let mut u = update(None, 248, 5, CanErrorState::BusOff);
        u.lec = Some(lec::BIT_RECESSIVE);
        dev.timed_apply(&u);
        assert_eq!(
            rd(&dev, REG_ESR) & 0x00FF_00FF,
            0x00F8_0047,
            "BOFF|EPVF|EWGF, TEC 248"
        );
        // Success clears LEC (RM0008: "cleared to 0 when a message has been
        // transferred without error").
        let mut u = update(None, 0, 0, CanErrorState::Active);
        u.lec = Some(lec::NO_ERROR);
        dev.timed_apply(&u);
        assert_eq!(rd(&dev, REG_ESR), 0);
    }

    #[test]
    fn timed_interrupts_follow_ier() {
        let mut dev = timed(BTR_500K_AT_36M);
        install_std_accept_filter(&mut dev, 0x111);
        let irqs = |dev: &mut BxCan| dev.tick().explicit_irqs.unwrap_or_default();
        assert!(irqs(&mut dev).is_empty());
        wr(
            &mut dev,
            REG_IER,
            IER_TMEIE | IER_FMPIE0 | IER_ERRIE | IER_EPVIE | IER_LECIE,
        );
        // RX0 (USB_LP_CAN_RX0, IRQ 20) while FIFO0 holds a message.
        let mut u = update(None, 0, 0, CanErrorState::Active);
        u.rx = Some(CanFrame::classic(0x111, vec![9]));
        dev.timed_apply(&u);
        assert_eq!(irqs(&mut dev), vec![20]);
        wr(&mut dev, REG_RF0R, RF_RFOM);
        assert!(irqs(&mut dev).is_empty());
        // TX (USB_HP_CAN_TX, IRQ 19) on RQCPx.
        send_mb(&mut dev, 0, 0x100, &[]);
        dev.timed_apply(&update(
            Some((0, CanTxOutcome::Ok)),
            0,
            0,
            CanErrorState::Active,
        ));
        assert_eq!(irqs(&mut dev), vec![19]);
        wr(&mut dev, REG_TSR, 1); // RQCP0 w1c
        assert!(irqs(&mut dev).is_empty());
        // SCE (CAN_SCE, IRQ 22): ERRI on an enabled ESR flag.
        dev.timed_apply(&update(None, 128, 0, CanErrorState::Passive));
        assert_ne!(rd(&dev, REG_MSR) & MSR_ERRI, 0);
        assert_eq!(irqs(&mut dev), vec![22]);
        wr(&mut dev, REG_MSR, MSR_ERRI);
        assert!(irqs(&mut dev).is_empty());
        // A flag whose enable is off sets no ERRI: EWGF without EWGIE.
        let mut quiet = timed(BTR_500K_AT_36M);
        wr(&mut quiet, REG_IER, IER_ERRIE | IER_BOFIE);
        quiet.timed_apply(&update(None, 100, 0, CanErrorState::Active));
        assert_eq!(rd(&quiet, REG_MSR) & MSR_ERRI, 0);
        let _ = IER_EWGIE;
        // Levels are reported for all four bxCAN lines.
        let mut lines = Vec::new();
        quiet.irq_line_levels(&mut |irq, level| lines.push((irq, level)));
        assert_eq!(
            lines,
            vec![(19, false), (20, false), (21, false), (22, false)]
        );
    }

    #[test]
    fn timed_bus_off_recovery_is_armed_by_abom_or_an_init_cycle() {
        // RM0008 §24.7.6: with ABOM the recovery starts at bus-off; without
        // it, firmware must enter and leave initialization mode.
        let mut dev = timed(BTR_500K_AT_36M);
        dev.clock.as_ref().unwrap().publish(500);
        dev.timed_apply(&update(None, 248, 0, CanErrorState::BusOff));
        assert_eq!(dev.timed_poll(36_000_000).recovery_armed_cycle, None);
        dev.clock.as_ref().unwrap().publish(900);
        wr(&mut dev, REG_MCR, MCR_INRQ);
        assert_eq!(
            dev.timed_poll(36_000_000).recovery_armed_cycle,
            None,
            "not in init"
        );
        wr(&mut dev, REG_MCR, 0);
        assert_eq!(dev.timed_poll(36_000_000).recovery_armed_cycle, Some(900));
        assert_eq!(dev.timed_poll(36_000_000).recovery_occurrences, 128);
        dev.timed_apply(&update(None, 0, 0, CanErrorState::Active));
        assert_eq!(dev.timed_poll(36_000_000).recovery_armed_cycle, None);
        assert_eq!(rd(&dev, REG_ESR) & 0b111, 0);

        let mut abom = timed(BTR_500K_AT_36M);
        wr(&mut abom, REG_MCR, 1 << 6);
        abom.clock.as_ref().unwrap().publish(700);
        abom.timed_apply(&update(None, 248, 0, CanErrorState::BusOff));
        assert_eq!(abom.timed_poll(36_000_000).recovery_armed_cycle, Some(700));
    }

    #[test]
    fn timed_mode_keeps_the_walk_on() {
        let mut dev = BxCan::new();
        assert!(!dev.needs_legacy_walk());
        dev.attach_timed().unwrap();
        assert!(dev.needs_legacy_walk());
        assert!(dev.is_timed());
        assert!(dev.attach_timed().is_err(), "attached once");
    }
}
