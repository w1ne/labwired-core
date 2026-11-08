// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA CAN (the 32-mailbox CAN module of the RA4M1/RA6M1-M3),
//! normal mailbox mode.
//!
//! Register layout from FSP `R7FA4M1AB.h` (`R_CAN0_Type`, 0x85C bytes; CAN0
//! at 0x40050000): `MB[32]` +0x200 (16 bytes each: `ID` +0, `DL` +4, `D[8]`
//! +6, `TS` +0xE), `MKR[8]` +0x400, `FIDCR[2]` +0x420, `MKIVLR` +0x428,
//! `MIER` +0x42C, `MCTL[32]` +0x820 (bytes), `CTLR` +0x840 (16-bit), `STR`
//! +0x842, `BCR` +0x844 (32-bit), `RFCR`/`RFPCR`/`TFCR`/`TFPCR` +0x848..B,
//! `EIER` +0x84C, `EIFR` +0x84D, `RECR` +0x84E, `TECR` +0x84F, `ECSR`
//! +0x850, `CSSR` +0x851, `MSSR` +0x852, `MSMR` +0x853, `TSR` +0x854,
//! `AFSR` +0x856, `TCR` +0x858.
//!
//! Modelled:
//! - `CTLR.CANM` and `SLPM` move the module between operation, reset, halt
//!   and sleep at once; `STR.RSTST` / `HLTST` / `SLPST` follow. Reset
//!   value `CTLR` = `STR` = 0x0500 (reset mode, sleep). Entering reset
//!   mode clears `MCTL0..31`, `EIFR`, `ECSR` (except `EDPM`), `MSMR` and
//!   `TCR`, as on silicon; mailboxes, masks and `BCR` are kept.
//! - `TCR` is stored only in halt mode (silicon ignores it elsewhere).
//!   `TSTE` with `TSTM` = 10 (external loopback) or 11 (internal loopback)
//!   makes a transmitted frame come back to this module's own receive
//!   mailboxes, acknowledged.
//! - Transmission: an `MCTLj.TRMREQ` mailbox in operation mode sends
//!   `MBj` (frame format from `CTLR.IDFM`: standard, extended, or mixed by
//!   `ID.IDE`; `ID.RTR`; `DL.DLC`, 9..15 meaning 8 data bytes). Pending
//!   mailboxes go out in `CTLR.TPM` order (lowest ID first, or lowest
//!   mailbox first). In loopback the frame completes at once: `SENTDATA`
//!   sets and `TRMREQ` stays set until software clears both.
//! - Reception: the lowest-numbered `RECREQ` mailbox whose `ID` matches
//!   (same frame format and `RTR`; the `SID`/`EID` bits under `MKR[j/4]`,
//!   or every bit when `MKIVLRj` is set) takes the frame: `ID`, `DL`,
//!   `D[0..8]`, `NEWDATA`. A mailbox that still holds unread data is
//!   overwritten (`CTLR.MLM` = 0) or keeps it (`MLM` = 1); both set
//!   `MSGLOST`.
//! - `MCTLj` flags (`SENTDATA`/`NEWDATA`, `TRMABT`/`MSGLOST`) clear on a 0
//!   write; clearing `TRMREQ` on a mailbox that has not sent sets `TRMABT`.
//! - `STR.NDST` / `SDST` / `NMLST` / `TABST` / `EST`, and the `MSMR` = 0..2
//!   mailbox search in `MSSR`, are derived from the mailboxes.
//!
//! No bus is attached. A transmission outside loopback gets no ACK: the
//! mailbox stays `TRMACTIVE` and `ECSR.AEF` / `EIFR.BEIF` set; the error
//! counters (`RECR`, `TECR`) stay 0 and the module never goes error
//! passive. Bit timing (`BCR`) is stored only, and the time stamp (`TSR`,
//! `MBj.TS`) reads 0.
//!
//! Refused rather than ignored: every interrupt (`MIER`, `EIER`), FIFO
//! mailbox mode (`CTLR.MBM`, `RFCR.RFE`, `TFCR.TFE`), the reserved
//! `CTLR.IDFM` = 11, listen-only transmission (`TSTM` = 01), a mailbox with
//! both `TRMREQ` and `RECREQ`, the channel search (`MSMR` = 11), and the
//! acceptance filter support register (`AFSR`).

use crate::{Peripheral, PeripheralTickResult, SimResult, SimulationError};
use std::any::Any;

const MB: usize = 0x200;
const MKR: usize = 0x400;
const MKIVLR: usize = 0x428;
const MIER: usize = 0x42C;
const MCTL: usize = 0x820;
const CTLR: usize = 0x840;
const STR: usize = 0x842;
const BCR: usize = 0x844;
const RFCR: usize = 0x848;
const RFPCR: usize = 0x849;
const TFCR: usize = 0x84A;
const TFPCR: usize = 0x84B;
const EIER: usize = 0x84C;
const EIFR: usize = 0x84D;
const RECR: usize = 0x84E;
const TECR: usize = 0x84F;
const ECSR: usize = 0x850;
const CSSR: usize = 0x851;
const MSSR: usize = 0x852;
const MSMR: usize = 0x853;
const TSR: usize = 0x854;
const AFSR: usize = 0x856;
const TCR: usize = 0x858;
const SIZE: usize = 0x85C;

const CTLR_RESET: u16 = 0x0500;
const CTLR_MBM: u16 = 1 << 0;
const CTLR_MLM: u16 = 1 << 3;
const CTLR_TPM: u16 = 1 << 4;
const CTLR_SLPM: u16 = 1 << 10;

const MCTL_DONE: u8 = 1 << 0; // SENTDATA / NEWDATA
const MCTL_ACTIVE: u8 = 1 << 1; // TRMACTIVE / INVALDATA
const MCTL_LOST: u8 = 1 << 2; // TRMABT / MSGLOST
const MCTL_RECREQ: u8 = 1 << 6;
const MCTL_TRMREQ: u8 = 1 << 7;
/// ONESHOT, RECREQ and TRMREQ: stored as written.
const MCTL_CONTROL: u8 = 0xD0;

const STR_NDST: u16 = 1 << 0;
const STR_SDST: u16 = 1 << 1;
const STR_NMLST: u16 = 1 << 4;
const STR_TABST: u16 = 1 << 6;
const STR_EST: u16 = 1 << 7;
const STR_RSTST: u16 = 1 << 8;
const STR_HLTST: u16 = 1 << 9;
const STR_SLPST: u16 = 1 << 10;

const EIFR_BEIF: u8 = 1 << 0;
const ECSR_AEF: u8 = 1 << 2;
const ECSR_EDPM: u8 = 1 << 7;
const TCR_TSTE: u8 = 1 << 0;

/// The 29 identifier bits of `MBj.ID` / `MKRn` (`SID` 28:18, `EID` 17:0).
const ID_BITS: u32 = 0x1FFF_FFFF;
const SID_BITS: u32 = 0x1FFC_0000;
const ID_RTR: u32 = 1 << 30;
const ID_IDE: u32 = 1 << 31;

/// One frame on the (looped-back) bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaCanFrame {
    /// 11-bit standard or 29-bit extended identifier.
    pub id: u32,
    pub extended: bool,
    pub remote: bool,
    pub dlc: u8,
    pub data: [u8; 8],
}

impl RaCanFrame {
    /// The identifier in `MBj.ID` bit positions (`SID` at 28:18).
    fn id_field(&self) -> u32 {
        if self.extended {
            self.id & ID_BITS
        } else {
            (self.id & 0x7FF) << 18
        }
    }
}

#[derive(Debug)]
pub struct RaCan {
    /// The whole register window, little-endian; `CTLR`, `MCTL`, `STR`,
    /// `MSSR` and the counters are kept apart.
    regs: Vec<u8>,
    ctlr: u16,
    mctl: [u8; 32],
    /// Evidence: frames sent (capped) and the counts.
    sent: Vec<RaCanFrame>,
    tx_frames: u64,
    rx_frames: u64,
}

impl Default for RaCan {
    fn default() -> Self {
        Self::new()
    }
}

impl RaCan {
    pub fn new() -> Self {
        Self {
            regs: vec![0; SIZE],
            ctlr: CTLR_RESET,
            mctl: [0; 32],
            sent: Vec::new(),
            tx_frames: 0,
            rx_frames: 0,
        }
    }

    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "RA CAN: {what} is not modelled"
        )))
    }

    fn u32_at(&self, o: usize) -> u32 {
        u32::from_le_bytes(self.regs[o..o + 4].try_into().unwrap())
    }

    fn set_u32_at(&mut self, o: usize, v: u32) {
        self.regs[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn canm(&self) -> u16 {
        (self.ctlr >> 8) & 3
    }

    fn in_reset(&self) -> bool {
        self.canm() & 1 != 0
    }

    fn operating(&self) -> bool {
        self.canm() == 0 && self.ctlr & CTLR_SLPM == 0
    }

    fn idfm(&self) -> u16 {
        (self.ctlr >> 1) & 3
    }

    /// `TCR.TSTE` with `TSTM` = 10 or 11.
    fn loopback(&self) -> bool {
        let tcr = self.regs[TCR];
        tcr & TCR_TSTE != 0 && (tcr >> 1) & 3 >= 2
    }

    fn str_reg(&self) -> u16 {
        let mut v = 0;
        for m in self.mctl {
            if m & MCTL_RECREQ != 0 {
                if m & MCTL_DONE != 0 {
                    v |= STR_NDST;
                }
                if m & MCTL_LOST != 0 {
                    v |= STR_NMLST;
                }
            } else {
                if m & MCTL_DONE != 0 {
                    v |= STR_SDST;
                }
                if m & MCTL_LOST != 0 {
                    v |= STR_TABST;
                }
            }
        }
        if self.regs[EIFR] != 0 {
            v |= STR_EST;
        }
        if self.in_reset() {
            v |= STR_RSTST;
        }
        if self.canm() == 2 {
            v |= STR_HLTST;
        }
        if self.ctlr & CTLR_SLPM != 0 {
            v |= STR_SLPST;
        }
        v
    }

    /// `MSSR` for `MSMR` = 0 (receive), 1 (transmit) or 2 (message lost).
    fn mssr(&self) -> u8 {
        let want = match self.regs[MSMR] & 3 {
            0 => |m: u8| m & MCTL_RECREQ != 0 && m & MCTL_DONE != 0,
            1 => |m: u8| m & MCTL_TRMREQ != 0 && m & MCTL_DONE != 0,
            _ => |m: u8| m & MCTL_RECREQ != 0 && m & MCTL_LOST != 0,
        };
        match self.mctl.iter().position(|&m| want(m)) {
            Some(j) => j as u8,
            None => 0x80,
        }
    }

    fn write_ctlr(&mut self, v: u16) -> SimResult<()> {
        if v & CTLR_MBM != 0 {
            return Self::refuse("FIFO mailbox mode (CTLR.MBM)");
        }
        if (v >> 1) & 3 == 3 {
            return Self::refuse("the reserved ID format CTLR.IDFM = 11");
        }
        let was_reset = self.in_reset();
        self.ctlr = v & 0x3FFF;
        if self.in_reset() && !was_reset {
            self.mctl = [0; 32];
            self.regs[EIFR] = 0;
            self.regs[ECSR] &= ECSR_EDPM;
            self.regs[MSMR] = 0;
            self.regs[TCR] = 0;
        }
        self.service()
    }

    fn write_mctl(&mut self, j: usize, v: u8) -> SimResult<()> {
        if v & MCTL_TRMREQ != 0 && v & MCTL_RECREQ != 0 {
            return Self::refuse("a mailbox with both MCTL.TRMREQ and RECREQ");
        }
        let old = self.mctl[j];
        let mut new = (old & v & (MCTL_DONE | MCTL_LOST)) | (v & MCTL_CONTROL);
        if old & MCTL_TRMREQ != 0 && v & MCTL_TRMREQ == 0 && old & MCTL_DONE == 0 {
            // A request withdrawn before it went out is an abort.
            new |= MCTL_LOST;
        }
        self.mctl[j] = new;
        self.service()
    }

    fn mb_frame(&self, j: usize) -> RaCanFrame {
        let o = MB + 16 * j;
        let id = self.u32_at(o);
        let extended = match self.idfm() {
            0 => false,
            1 => true,
            _ => id & ID_IDE != 0,
        };
        let mut data = [0u8; 8];
        data.copy_from_slice(&self.regs[o + 6..o + 14]);
        RaCanFrame {
            id: if extended {
                id & ID_BITS
            } else {
                (id >> 18) & 0x7FF
            },
            extended,
            remote: id & ID_RTR != 0,
            dlc: self.regs[o + 4] & 0xF,
            data,
        }
    }

    /// Send every pending transmit mailbox (operation mode only).
    fn service(&mut self) -> SimResult<()> {
        if !self.operating() {
            return Ok(());
        }
        loop {
            let pending: Vec<usize> = (0..32)
                .filter(|&j| {
                    let m = self.mctl[j];
                    m & MCTL_TRMREQ != 0 && m & (MCTL_DONE | MCTL_ACTIVE) == 0
                })
                .collect();
            let next = if self.ctlr & CTLR_TPM != 0 {
                pending.first().copied()
            } else {
                // ID priority: lowest identifier wins arbitration, a data
                // frame beats a remote frame, then the lowest mailbox.
                pending.iter().copied().min_by_key(|&j| {
                    let f = self.mb_frame(j);
                    (f.id_field(), f.remote, j)
                })
            };
            let Some(j) = next else {
                return Ok(());
            };
            if self.regs[TCR] & TCR_TSTE != 0 && (self.regs[TCR] >> 1) & 3 == 1 {
                return Self::refuse("a transmission in listen-only mode (TCR.TSTM = 01)");
            }
            if !self.loopback() {
                // Nothing on the bus acknowledges: the mailbox keeps
                // retrying.
                self.mctl[j] |= MCTL_ACTIVE;
                self.regs[ECSR] |= ECSR_AEF;
                self.regs[EIFR] |= EIFR_BEIF;
                continue;
            }
            let frame = self.mb_frame(j);
            self.mctl[j] |= MCTL_DONE;
            self.tx_frames += 1;
            if self.sent.len() < 64 {
                self.sent.push(frame);
            }
            self.receive(&frame);
        }
    }

    /// Store a frame in the first matching receive mailbox.
    pub fn receive(&mut self, frame: &RaCanFrame) -> bool {
        let idfm = self.idfm();
        if (idfm == 0 && frame.extended) || (idfm == 1 && !frame.extended) {
            return false;
        }
        let compare = if frame.extended { ID_BITS } else { SID_BITS };
        let fid = frame.id_field();
        let mkivlr = self.u32_at(MKIVLR);
        let hit = (0..32).find(|&k| {
            if self.mctl[k] & MCTL_RECREQ == 0 {
                return false;
            }
            let id = self.u32_at(MB + 16 * k);
            if idfm == 2 && (id & ID_IDE != 0) != frame.extended {
                return false;
            }
            if (id & ID_RTR != 0) != frame.remote {
                return false;
            }
            let mask = if mkivlr & (1 << k) != 0 {
                ID_BITS
            } else {
                self.u32_at(MKR + 4 * (k / 4)) & ID_BITS
            };
            (id ^ fid) & mask & compare == 0
        });
        let Some(k) = hit else {
            return false;
        };
        if self.mctl[k] & MCTL_DONE != 0 {
            self.mctl[k] |= MCTL_LOST;
            if self.ctlr & CTLR_MLM != 0 {
                return true;
            }
        }
        let o = MB + 16 * k;
        let mut id = (self.u32_at(o) & !(compare | ID_RTR)) | fid;
        if frame.remote {
            id |= ID_RTR;
        }
        if idfm == 2 {
            id = (id & !ID_IDE) | if frame.extended { ID_IDE } else { 0 };
        }
        self.set_u32_at(o, id);
        self.regs[o + 4] = frame.dlc & 0xF;
        self.regs[o + 5] = 0;
        self.regs[o + 6..o + 14].copy_from_slice(&frame.data);
        self.regs[o + 14] = 0;
        self.regs[o + 15] = 0;
        self.mctl[k] |= MCTL_DONE;
        self.rx_frames += 1;
        true
    }

    fn reg(&self, o: usize) -> Option<u8> {
        Some(match o {
            MCTL..=0x83F => self.mctl[o - MCTL],
            CTLR => self.ctlr as u8,
            0x841 => (self.ctlr >> 8) as u8,
            STR => self.str_reg() as u8,
            0x843 => (self.str_reg() >> 8) as u8,
            // Both FIFOs are always empty (RFEST / TFEST).
            RFCR | TFCR => 0x80,
            RFPCR | TFPCR | RECR | TECR | TSR | 0x855 => 0,
            MSSR => self.mssr(),
            AFSR | 0x857 => return None,
            0x859..=0x85B => 0,
            MB..=0x42F | BCR..=0x847 | EIER | EIFR | ECSR | CSSR | MSMR | TCR => self.regs[o],
            _ => return None,
        })
    }

    fn write_reg(&mut self, o: usize, v: u8) -> SimResult<()> {
        match o {
            MIER..=0x42F if v != 0 => return Self::refuse("mailbox interrupts (MIER)"),
            MCTL..=0x83F => return self.write_mctl(o - MCTL, v),
            CTLR => return self.write_ctlr((self.ctlr & 0xFF00) | v as u16),
            0x841 => return self.write_ctlr((self.ctlr & 0x00FF) | (v as u16) << 8),
            STR | 0x843 | RECR | TECR | MSSR | TSR | 0x855 | RFPCR | TFPCR | 0x859..=0x85B => {}
            RFCR if v & 1 != 0 => return Self::refuse("the receive FIFO (RFCR.RFE)"),
            TFCR if v & 1 != 0 => return Self::refuse("the transmit FIFO (TFCR.TFE)"),
            RFCR | TFCR => {}
            EIER if v != 0 => return Self::refuse("error interrupts (EIER)"),
            EIFR => self.regs[EIFR] &= v,
            ECSR => self.regs[ECSR] = (self.regs[ECSR] & v & !ECSR_EDPM) | (v & ECSR_EDPM),
            MSMR if v & 3 == 3 => return Self::refuse("the channel search (MSMR = 11)"),
            AFSR | 0x857 => return Self::refuse("the acceptance filter support register (AFSR)"),
            TCR => {
                if self.canm() == 2 {
                    self.regs[TCR] = v & 0x07;
                }
            }
            MB..=0x42F | BCR..=0x847 | EIER | CSSR | MSMR => self.regs[o] = v,
            _ => return Self::refuse("a store outside the register map"),
        }
        Ok(())
    }

    /// Frames this module has sent (the first 64).
    pub fn sent_frames(&self) -> &[RaCanFrame] {
        &self.sent
    }
}

impl Peripheral for RaCan {
    fn read(&self, o: u64) -> SimResult<u8> {
        match self.reg(o as usize) {
            Some(v) => Ok(v),
            None => Self::refuse("a load outside the register map"),
        }
    }
    fn peek(&self, o: u64) -> Option<u8> {
        self.reg(o as usize)
    }
    fn write(&mut self, o: u64, v: u8) -> SimResult<()> {
        self.write_reg(o as usize, v)
    }
    fn write_u16(&mut self, o: u64, v: u16) -> SimResult<()> {
        if o as usize == CTLR {
            return self.write_ctlr(v);
        }
        self.write_reg(o as usize, v as u8)?;
        self.write_reg(o as usize + 1, (v >> 8) as u8)
    }
    fn write_u32(&mut self, o: u64, v: u32) -> SimResult<()> {
        if o as usize == CTLR {
            // CTLR + STR: STR is read-only.
            return self.write_ctlr(v as u16);
        }
        // A word store into MCTL lands four mailboxes at once; `service`
        // runs after each, which is the order silicon would see them in.
        for i in 0..4 {
            self.write_reg(o as usize + i, (v >> (8 * i)) as u8)?;
        }
        Ok(())
    }
    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }
    fn on_node_reset(&mut self) {
        let (sent, tx, rx) = (
            std::mem::take(&mut self.sent),
            self.tx_frames,
            self.rx_frames,
        );
        *self = Self::new();
        self.sent = sent;
        self.tx_frames = tx;
        self.rx_frames = rx;
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        let sent: Vec<_> = self
            .sent
            .iter()
            .map(|f| {
                serde_json::json!({"id": f.id, "extended": f.extended, "remote": f.remote,
                    "dlc": f.dlc, "data": f.data})
            })
            .collect();
        serde_json::json!({"peripheral": "ra_can", "ctlr": self.ctlr, "str": self.str_reg(),
            "tcr": self.regs[TCR], "bcr": self.u32_at(BCR), "mctl": self.mctl.to_vec(),
            "tx_frames": self.tx_frames, "rx_frames": self.rx_frames, "sent": sent})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w8(c: &mut RaCan, o: usize, v: u8) {
        c.write(o as u64, v).unwrap();
    }
    fn w16(c: &mut RaCan, o: usize, v: u16) {
        c.write_u16(o as u64, v).unwrap();
    }
    fn w32(c: &mut RaCan, o: usize, v: u32) {
        c.write_u32(o as u64, v).unwrap();
    }
    fn r8(c: &RaCan, o: usize) -> u8 {
        c.read(o as u64).unwrap()
    }
    fn r16(c: &RaCan, o: usize) -> u16 {
        c.read_u16(o as u64).unwrap()
    }
    fn r32(c: &RaCan, o: usize) -> u32 {
        c.read_u32(o as u64).unwrap()
    }

    /// The RA4M1 hardware-manual start-up: leave sleep, configure in reset,
    /// select the test mode in halt, then operate.
    fn bring_up(c: &mut RaCan, ctlr_cfg: u16, tcr: u8) {
        w16(c, CTLR, 0x0100); // CANM = reset, SLPM = 0
        w32(c, BCR, 0x3002_0300 | 0x1);
        w16(c, CTLR, 0x0100 | ctlr_cfg);
        w16(c, CTLR, 0x0200 | ctlr_cfg); // halt
        w8(c, TCR, tcr);
        w32(c, MKIVLR, 0);
        for n in 0..8 {
            w32(c, MKR + 4 * n, ID_BITS);
        }
    }

    fn operate(c: &mut RaCan, ctlr_cfg: u16) {
        w16(c, CTLR, ctlr_cfg);
    }

    fn load_tx(c: &mut RaCan, j: usize, id: u32, data: &[u8]) {
        let o = MB + 16 * j;
        w32(c, o, id);
        w16(c, o + 4, data.len() as u16);
        for (i, b) in data.iter().enumerate() {
            w8(c, o + 6 + i, *b);
        }
    }

    #[test]
    fn reset_values() {
        let c = RaCan::new();
        assert_eq!(r16(&c, CTLR), 0x0500);
        assert_eq!(r16(&c, STR), 0x0500);
        assert_eq!(r8(&c, RFCR), 0x80);
        assert_eq!(r8(&c, MSSR), 0x80);
    }

    #[test]
    fn mode_bits_follow_canm_and_slpm() {
        let mut c = RaCan::new();
        w16(&mut c, CTLR, 0x0100);
        assert_eq!(r16(&c, STR) & 0x0700, STR_RSTST);
        w16(&mut c, CTLR, 0x0200);
        assert_eq!(r16(&c, STR) & 0x0700, STR_HLTST);
        w16(&mut c, CTLR, 0x0000);
        assert_eq!(r16(&c, STR) & 0x0700, 0);
        w16(&mut c, CTLR, 0x0300);
        assert_eq!(r16(&c, STR) & 0x0700, STR_RSTST);
    }

    #[test]
    fn internal_loopback_delivers_a_standard_frame() {
        let mut c = RaCan::new();
        bring_up(&mut c, 0, 0x07);
        load_tx(&mut c, 0, 0x123 << 18, &[0xA5]);
        w32(&mut c, MB + 16, 0x123 << 18);
        operate(&mut c, 0);
        w8(&mut c, MCTL + 1, MCTL_RECREQ);
        w8(&mut c, MCTL, MCTL_TRMREQ);
        assert_eq!(r8(&c, MCTL), MCTL_TRMREQ | MCTL_DONE);
        assert_eq!(r8(&c, MCTL + 1), MCTL_RECREQ | MCTL_DONE);
        assert_eq!((r32(&c, MB + 16) >> 18) & 0x7FF, 0x123);
        assert_eq!(r16(&c, MB + 16 + 4), 1);
        assert_eq!(r8(&c, MB + 16 + 6), 0xA5);
        assert_eq!(r16(&c, STR) & (STR_NDST | STR_SDST), STR_NDST | STR_SDST);
        // Mailbox search: receive mode finds MB1, transmit mode MB0.
        assert_eq!(r8(&c, MSSR), 1);
        w8(&mut c, MSMR, 1);
        assert_eq!(r8(&c, MSSR), 0);
        // Software clears the flags with 0 writes.
        w8(&mut c, MCTL, 0);
        w8(&mut c, MCTL + 1, MCTL_RECREQ);
        assert_eq!(r8(&c, MCTL), 0);
        assert_eq!(r8(&c, MCTL + 1), MCTL_RECREQ);
        assert_eq!(c.sent_frames().len(), 1);
    }

    #[test]
    fn extended_frames_and_mixed_mode_match_on_ide() {
        let mut c = RaCan::new();
        let mixed = 2 << 1;
        bring_up(&mut c, mixed, 0x05);
        load_tx(&mut c, 0, ID_IDE | 0x1ABC_DEF0, &[1, 2, 3, 4]);
        // MB1 waits for a standard frame with the same SID; MB2 for the
        // extended one.
        w32(&mut c, MB + 16, 0x1ABC_DEF0 & SID_BITS);
        w32(&mut c, MB + 32, ID_IDE | 0x1ABC_DEF0);
        operate(&mut c, mixed);
        w8(&mut c, MCTL + 1, MCTL_RECREQ);
        w8(&mut c, MCTL + 2, MCTL_RECREQ);
        w8(&mut c, MCTL, MCTL_TRMREQ);
        assert_eq!(r8(&c, MCTL + 1) & MCTL_DONE, 0);
        assert_eq!(r8(&c, MCTL + 2) & MCTL_DONE, MCTL_DONE);
        assert_eq!(r32(&c, MB + 32 + 6), 0x0403_0201);
    }

    #[test]
    fn masks_and_mask_invalid() {
        let mut c = RaCan::new();
        bring_up(&mut c, 0, 0x07);
        // MB4..7 share MKR[1]; mask only SID[10:4]. MB4 accepts 0x120..0x12F.
        w32(&mut c, MKR + 4, 0x7F0 << 18);
        w32(&mut c, MB + 16 * 4, 0x120 << 18);
        // MB5 has the same mask group but MKIVLR makes it exact.
        w32(&mut c, MB + 16 * 5, 0x120 << 18);
        w32(&mut c, MKIVLR, 1 << 5);
        load_tx(&mut c, 0, 0x12B << 18, &[9]);
        operate(&mut c, 0);
        w8(&mut c, MCTL + 5, MCTL_RECREQ);
        w8(&mut c, MCTL, MCTL_TRMREQ);
        assert_eq!(r8(&c, MCTL + 5) & MCTL_DONE, 0, "exact match refuses 0x12B");
        w8(&mut c, MCTL, 0);
        w8(&mut c, MCTL + 4, MCTL_RECREQ);
        w8(&mut c, MCTL, MCTL_TRMREQ);
        assert_eq!(r8(&c, MCTL + 4) & MCTL_DONE, MCTL_DONE);
        assert_eq!((r32(&c, MB + 16 * 4) >> 18) & 0x7FF, 0x12B);
    }

    #[test]
    fn overwrite_and_overrun_set_msglost() {
        for (mlm, expect) in [(0u16, 0x22u8), (CTLR_MLM, 0x11u8)] {
            let mut c = RaCan::new();
            bring_up(&mut c, mlm, 0x07);
            w32(&mut c, MB + 16, 0x100 << 18);
            operate(&mut c, mlm);
            w8(&mut c, MCTL + 1, MCTL_RECREQ);
            for d in [0x11u8, 0x22] {
                load_tx(&mut c, 0, 0x100 << 18, &[d]);
                w8(&mut c, MCTL, 0);
                w8(&mut c, MCTL, MCTL_TRMREQ);
            }
            assert_eq!(r8(&c, MB + 16 + 6), expect);
            assert_eq!(r8(&c, MCTL + 1) & MCTL_LOST, MCTL_LOST);
            assert_eq!(r16(&c, STR) & STR_NMLST, STR_NMLST);
        }
    }

    #[test]
    fn id_priority_orders_pending_mailboxes() {
        let mut c = RaCan::new();
        bring_up(&mut c, 0, 0x07);
        load_tx(&mut c, 0, 0x300 << 18, &[]);
        load_tx(&mut c, 1, 0x100 << 18, &[]);
        // Requested in halt mode: both wait for operation mode.
        w8(&mut c, MCTL, MCTL_TRMREQ);
        w8(&mut c, MCTL + 1, MCTL_TRMREQ);
        assert!(c.sent_frames().is_empty());
        operate(&mut c, 0);
        let ids: Vec<u32> = c.sent_frames().iter().map(|f| f.id).collect();
        assert_eq!(ids, [0x100, 0x300]);
    }

    #[test]
    fn no_bus_means_no_ack() {
        let mut c = RaCan::new();
        bring_up(&mut c, 0, 0x00);
        load_tx(&mut c, 0, 0x123 << 18, &[0xA5]);
        operate(&mut c, 0);
        w8(&mut c, MCTL, MCTL_TRMREQ);
        assert_eq!(r8(&c, MCTL), MCTL_TRMREQ | MCTL_ACTIVE);
        assert_eq!(r8(&c, ECSR) & ECSR_AEF, ECSR_AEF);
        assert_eq!(r16(&c, STR) & STR_EST, STR_EST);
        // Withdrawing the request aborts it.
        w8(&mut c, MCTL, 0);
        assert_eq!(r8(&c, MCTL) & MCTL_LOST, MCTL_LOST);
        assert!(c.sent_frames().is_empty());
    }

    #[test]
    fn tcr_only_takes_in_halt_and_reset_clears_state() {
        let mut c = RaCan::new();
        w16(&mut c, CTLR, 0x0100);
        w8(&mut c, TCR, 0x07);
        assert_eq!(r8(&c, TCR), 0);
        bring_up(&mut c, 0, 0x07);
        assert_eq!(r8(&c, TCR), 0x07);
        w8(&mut c, MCTL + 3, MCTL_RECREQ);
        w16(&mut c, CTLR, 0x0100);
        assert_eq!(r8(&c, TCR), 0);
        assert_eq!(r8(&c, MCTL + 3), 0);
    }

    #[test]
    fn refusals() {
        let mut c = RaCan::new();
        assert!(c.write_u16(CTLR as u64, 0x0101).is_err(), "FIFO mode");
        assert!(c.write_u16(CTLR as u64, 0x0106).is_err(), "IDFM = 11");
        assert!(c.write_u32(MIER as u64, 1).is_err());
        assert!(c.write(EIER as u64, 1).is_err());
        assert!(c.write(RFCR as u64, 1).is_err());
        assert!(c.write(TFCR as u64, 1).is_err());
        assert!(c.write(MSMR as u64, 3).is_err());
        assert!(c.write_u16(AFSR as u64, 0x1234).is_err());
        assert!(c.write(MCTL as u64, 0xC0).is_err());
        assert!(c.read(0x85C).is_err());
        // Listen-only: a transmission request is refused.
        bring_up(&mut c, 0, 0x03);
        load_tx(&mut c, 0, 0, &[]);
        operate(&mut c, 0);
        assert!(c.write(MCTL as u64, MCTL_TRMREQ).is_err());
    }
}
