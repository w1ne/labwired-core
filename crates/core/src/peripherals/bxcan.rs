// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! bxCAN — STM32 basic-extended CAN controller.
//!
//! Identical layout across F1/F4/L4 (RM0351 §40 for L4). Models the master
//! control / status handshake (MCR.INRQ -> MSR.INAK, MCR.SLEEP -> MSR.SLAK)
//! and TSR mailbox-empty bits so HAL_CAN_Init can complete. Mailbox / filter
//! storage is preserved as raw u32 reads-back.

use crate::SimResult;
use std::collections::HashMap;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BxCan {
    mcr: u32,
    msr: u32,
    tsr: u32,
    rfr0: u32,
    rfr1: u32,
    ier: u32,
    esr: u32,
    btr: u32,
    /// Mailboxes, filter banks, fifo data — register access pass-through.
    extra: HashMap<u64, u32>,
}

impl BxCan {
    pub fn new() -> Self {
        Self {
            // INRQ deasserted, SLEEP set per reset value (RM0351 §40.7.2).
            mcr: 0x0001_0002,
            // Reset state per RM0351 §40.8.2: SLAK=1 (asleep), SAMP=1 (last
            // sample point read as recessive). RX bit (11) is the live state
            // of the CAN_RX pin and reads recessive (=1) when the bus is idle
            // — but on a NUCLEO with no CAN transceiver wired, the line
            // floats and silicon reports RX=0. Captured value: 0x0000_040A
            // (SLAK + SAMP) at reset; after INRQ=1 firmware sees 0x0000_0409
            // (SLAK clears, INAK + WKUI + SAMP).
            msr: 0x0000_040A,
            // All 3 TX mailboxes empty (TME0/1/2 = 1) -> bits 26,27,28 = 1.
            tsr: 0x1C00_0000,
            rfr0: 0,
            rfr1: 0,
            ier: 0,
            esr: 0,
            btr: 0x0123_0000,
            extra: HashMap::new(),
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x000 => self.mcr,
            0x004 => self.msr,
            0x008 => self.tsr,
            0x00C => self.rfr0,
            0x010 => self.rfr1,
            0x014 => self.ier,
            0x018 => self.esr,
            0x01C => self.btr,
            other => self.extra.get(&other).copied().unwrap_or(0),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x000 => {
                self.mcr = value & 0x0001_FF7F;
                // INRQ -> INAK handshake (silicon polls within ~µs).
                // Setting INRQ also latches WKUI (bit 3) per silicon capture.
                if (self.mcr & 1) != 0 {
                    self.msr |= 1; // INAK
                    self.msr |= 1 << 3; // WKUI
                    self.msr &= !(1 << 1); // SLAK clears
                } else {
                    self.msr &= !1;
                }
                // SLEEP -> SLAK
                if (self.mcr & 2) != 0 {
                    self.msr |= 1 << 1;
                } else {
                    self.msr &= !(1 << 1);
                }
                // RESET (bit 15) self-clears
                self.mcr &= !(1 << 15);
            }
            0x004 => {
                // MSR is rc_w1 for ERRI/WKUI/SLAKI/INAKI (bits 0..4 actually).
                // For survival mode, accept the writes (clear matched flags).
                self.msr &= !(value & 0x0000_001C);
            }
            0x008 => {
                // TSR rc_w1 for RQCP, TXOK, ALST, TERR per mailbox.
                let mask = value & 0x000F_000F & 0x008F_008F & 0x0F00_0F00 | (value & 0x0F0F_0F0F);
                self.tsr &= !mask;
                // Re-assert TMEx after clear so subsequent transmits pass.
                self.tsr |= 0x1C00_0000;
            }
            0x00C => self.rfr0 = value & 0x37,
            0x010 => self.rfr1 = value & 0x37,
            0x014 => self.ier = value & 0x0000_FFFF,
            0x018 => {
                // ESR is mostly read-only, only LEC writable.
                self.esr = (self.esr & !0x70) | (value & 0x70);
            }
            0x01C => self.btr = value & 0xC37F_03FF,
            other => {
                self.extra.insert(other, value);
            }
        }
    }
}

impl Default for BxCan {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for BxCan {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
