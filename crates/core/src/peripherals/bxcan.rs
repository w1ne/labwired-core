// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! bxCAN — STM32 basic-extended CAN controller (F1/F4/L4, RM0008 §24).
//!
//! Models the master control / status handshake (MCR.INRQ -> MSR.INAK,
//! MCR.SLEEP -> MSR.SLAK), the three TX mailboxes, and RX FIFO0 with a
//! working **loopback** datapath (BTR.LBKM): a frame whose transmission is
//! requested via TIxR.TXRQ is decoded from the mailbox and, in loopback,
//! delivered straight into RX FIFO0 — exactly the path the on-chip ISO-TP
//! stack exercises for a self-test. Acceptance filtering and bit timing are
//! not modeled (every frame is accepted into FIFO0; transmission completes
//! immediately).
//!
//! Register reset values are silicon-pinned (STM32F103C8, RM0008 §24.9.2).

use crate::network::CanFrame;
use crate::peripherals::fdcan::FdcanTraceFrame;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::collections::HashMap;
use std::collections::VecDeque;

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

const MCR_INRQ: u32 = 1 << 0;
const MCR_SLEEP: u32 = 1 << 1;
const MCR_RESET: u32 = 1 << 15;

const TI_TXRQ: u32 = 1 << 0;
const TI_RTR: u32 = 1 << 1;
const TI_IDE: u32 = 1 << 2;

const RF_RFOM: u32 = 1 << 5; // release output mailbox (w1)

const BTR_LBKM: u32 = 1 << 30; // loopback mode

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
    /// Raw TX mailbox words, indexed by (offset - 0x180)/4 (12 words).
    tx_mb: [u32; 12],
    /// Filter banks and any other register-window storage (read-back).
    extra: HashMap<u64, u32>,
    /// RX FIFO0 contents (loopback / future network delivery).
    #[serde(skip)]
    rx_fifo0: VecDeque<CanFrame>,
    /// Frames transmitted with loopback off and no bus attached. Bounded.
    #[serde(skip)]
    pub tx_frames: VecDeque<CanFrame>,
    /// Frame-level CAN trace for the logic analyzer (tx + loopback rx), shared
    /// `FdcanTraceFrame` shape so the existing UDS/CAN decoder consumes it.
    #[serde(skip)]
    trace_seq: u64,
    #[serde(skip)]
    trace: VecDeque<FdcanTraceFrame>,
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
            tx_mb: [0; 12],
            extra: HashMap::new(),
            rx_fifo0: VecDeque::new(),
            tx_frames: VecDeque::new(),
            trace_seq: 0,
            trace: VecDeque::new(),
        }
    }

    /// Frame-level trace for the logic analyzer; mirrors the FDCAN shape so the
    /// shared CAN/UDS decoder works for both controllers.
    pub fn trace_snapshot(&self, peripheral: &str) -> Vec<FdcanTraceFrame> {
        self.trace
            .iter()
            .cloned()
            .map(|mut frame| {
                frame.peripheral = peripheral.to_string();
                frame
            })
            .collect()
    }

    fn push_trace(&mut self, direction: &'static str, frame: &CanFrame) {
        self.trace_seq = self.trace_seq.wrapping_add(1);
        if self.trace.len() >= 200 {
            self.trace.pop_front();
        }
        self.trace.push_back(FdcanTraceFrame {
            seq: self.trace_seq,
            peripheral: String::new(),
            direction: direction.to_string(),
            id: frame.id,
            data: frame.data.clone(),
            extended: frame.extended,
            fd: frame.fd,
            bitrate_switch: frame.bitrate_switch,
            remote: frame.remote,
        });
    }

    fn running(&self) -> bool {
        // Out of initialization (INRQ cleared) — frames may move.
        self.mcr & MCR_INRQ == 0
    }

    fn loopback(&self) -> bool {
        self.btr & BTR_LBKM != 0
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

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            REG_MCR => self.mcr,
            REG_MSR => self.msr,
            REG_TSR => self.tsr,
            REG_RF0R => self.rf0r(),
            REG_RF1R => self.rf1r,
            REG_IER => self.ier,
            REG_ESR => self.esr,
            REG_BTR => self.btr,
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
            0x8 => word_from_bytes(&frame.data, 0),  // RDL0R: data[0..3]
            0xC => word_from_bytes(&frame.data, 4),  // RDH0R: data[4..7]
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            REG_MCR => {
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
            }
            REG_MSR => self.msr &= !(value & 0x0000_001C), // rc_w1 flags
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
            REG_BTR => self.btr = value & 0xC37F_03FF,
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
        if !self.running() {
            return;
        }
        let base = (mailbox * 4) as usize; // 4 words per mailbox
        let Some(frame) = self.decode_tx_mailbox(base) else {
            return;
        };
        // Clear TXRQ (mailbox emptied) and flag completion for this mailbox.
        self.tx_mb[base] &= !TI_TXRQ;
        let mb = mailbox as u32;
        self.tsr |= (1 << (mb * 8)) | (1 << (mb * 8 + 1)); // RQCPx | TXOKx
        self.tsr |= TSR_TME_ALL;

        self.push_trace("tx", &frame);
        if self.loopback() {
            self.deliver_rx(frame);
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

    /// Deliver a frame into RX FIFO0 (loopback or external network). Returns
    /// false when the FIFO was full (FOVR0).
    pub fn deliver_rx(&mut self, frame: CanFrame) -> bool {
        if !self.running() {
            return false;
        }
        if self.rx_fifo0.len() >= FIFO0_DEPTH {
            return false;
        }
        self.push_trace("rx", &frame);
        self.rx_fifo0.push_back(frame);
        true
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

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::with_irq(false)
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

    /// Leave init with loopback enabled, mirroring HAL_CAN bring-up.
    fn enter_loopback(dev: &mut BxCan) {
        wr(dev, REG_MCR, MCR_INRQ); // request init
        wr(dev, REG_BTR, BTR_LBKM); // loopback
        wr(dev, REG_MCR, 0); // leave init -> running
    }

    /// Queue a standard-ID frame in TX mailbox 0 and request its transmission.
    fn send(dev: &mut BxCan, id: u32, data: &[u8]) {
        wr(dev, TX_MB_BASE + 0x08, word_from_bytes(data, 0)); // TDL0R
        wr(dev, TX_MB_BASE + 0x0C, word_from_bytes(data, 4)); // TDH0R
        wr(dev, TX_MB_BASE + 0x04, data.len() as u32); // TDT0R: DLC
        wr(dev, TX_MB_BASE + 0x00, (id & 0x7FF) << 21 | TI_TXRQ); // TI0R
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
        send(&mut dev, 0x111, &[0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33]);

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
    fn rfom_release_advances_the_fifo() {
        let mut dev = BxCan::new();
        enter_loopback(&mut dev);
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
        wr(&mut dev, REG_BTR, BTR_LBKM);
        send(&mut dev, 0x111, &[0x01, 0x02]);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0, "no delivery while INRQ set");
    }

    #[test]
    fn loopback_frames_are_exposed_as_can_trace_events() {
        let mut dev = BxCan::new();
        enter_loopback(&mut dev);
        send(&mut dev, 0x111, &[0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33]);
        let trace = dev.trace_snapshot("bxcan1");
        // One loopback frame appears as a tx then an rx event.
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].peripheral, "bxcan1");
        assert_eq!(trace[0].direction, "tx");
        assert_eq!(trace[0].id, 0x111);
        assert_eq!(trace[1].direction, "rx");
        assert_eq!(trace[1].id, 0x111);
        assert_eq!(trace[1].data, vec![0x10, 0x0B, 0x27, 0x01, 0x5A, 0x11, 0x22, 0x33]);
    }

    #[test]
    fn non_loopback_tx_is_buffered_not_received() {
        let mut dev = BxCan::new();
        wr(&mut dev, REG_MCR, 0); // running, no loopback
        send(&mut dev, 0x111, &[0x42]);
        assert_eq!(rd(&dev, REG_RF0R) & 0x3, 0, "not looped back");
        assert_eq!(dev.tx_frames.len(), 1);
        assert_eq!(dev.tx_frames[0].id, 0x111);
    }
}
