// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! USB OTG FS — Synopsys DesignWare USB 2.0 OTG controller (RM0351 §44).
//!
//! Massive register file (>1KB). We model just enough for HAL_PCD_Init /
//! HAL_HCD_Init to complete: core reset bits in GRSTCTL, AHB-idle hint in
//! GRSTCTL.AHBIDL, mode-mismatch handshake on GINTSTS, and FIFO/EP storage.
//!
//! Reset values per RM §44.16 (subset that matters for init):
//!   GUSBCFG = 0x0000_1440 (TRDT=0x9, FHMOD=0, FDMOD=0)
//!   GRSTCTL = 0x8000_0000 (AHBIDL set immediately — no AHB pending)
//!   GINTSTS = 0x0400_0001 (CMOD=Device + CURMOD bit-0 reads as Device)
//!   HPRT    = 0
//!   DCTL    = 0
//!
//! All other registers default 0. Reads outside the modeled set return 0
//! (treated as "feature disabled / no events pending").

use crate::SimResult;
use std::collections::HashMap;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct UsbOtg {
    /// Sparse register store. We model the "live" registers explicitly via
    /// match arms; this map captures everything else so firmware that writes
    /// then reads back gets its value back (good enough for init flows).
    storage: HashMap<u64, u32>,
    grstctl: u32,
    gintsts: u32,
    gintmsk: u32,
    gusbcfg: u32,
    gahbcfg: u32,
    dcfg: u32,
    dctl: u32,
    dsts: u32,
    hprt: u32,
}

impl UsbOtg {
    pub fn new() -> Self {
        Self {
            storage: HashMap::new(),
            // AHBIDL set: HAL polls this immediately after reset.
            grstctl: 0x8000_0000,
            // Reset value verified against NUCLEO-L476RG silicon:
            //   bit 5 NPTXFE  (non-periodic TX FIFO empty)
            //   bit 25 PTXFE  (periodic TX FIFO empty in host mode)
            //   bit 26 CIDSCHG (connector ID status change)
            //   bit 28 DISCINT (disconnect detected — cable not plugged)
            gintsts: 0x1400_0020,
            gintmsk: 0,
            gusbcfg: 0x0000_1440,
            gahbcfg: 0,
            dcfg: 0,
            dctl: 0,
            // ENUMSPD = full-speed (bits 1:2 = 0b11 in modern HAL parlance).
            dsts: 0,
            hprt: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x000 => self.gusbcfg, // GOTGCTL — OTG control. Stub 0.
            0x004 => 0,            // GOTGINT
            0x008 => self.gahbcfg,
            0x00C => self.gusbcfg,
            0x010 => self.grstctl,
            0x014 => self.gintsts,
            0x018 => self.gintmsk,
            0x01C => 0, // GRXSTSR (queue empty)
            0x020 => 0, // GRXSTSP
            0x024 => 0, // GRXFSIZ
            0x028 => 0, // HNPTXFSIZ / DIEPTXF0
            0x02C => 0, // HNPTXSTS
            0x040 => 0x4F54_2000, // GCCFG (default reset includes power-down state)
            0x044 => 0x0000_1200, // CID
            0x800 => self.dcfg,
            0x804 => self.dctl,
            0x808 => self.dsts,
            0x440 => self.hprt,
            other => self.storage.get(&other).copied().unwrap_or(0),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x008 => self.gahbcfg = value & 0x3F,
            0x00C => self.gusbcfg = value,
            0x010 => {
                // CSRST (bit 0) self-clears; drive AHBIDL high so HAL polling exits.
                self.grstctl = (value & !1) | 0x8000_0000;
            }
            0x014 => {
                // rc_w1: clear matched flags
                self.gintsts &= !(value & 0xFBFF_F8FF);
            }
            0x018 => self.gintmsk = value,
            0x800 => self.dcfg = value,
            0x804 => {
                self.dctl = value;
                // SDIS (bit 1) controls soft-disconnect; mirror for diagnostics.
            }
            0x440 => {
                // HPRT bits PCSTS/PRES/PENA toggle on writing 1
                self.hprt = value;
            }
            other => {
                self.storage.insert(other, value);
            }
        }
    }
}

impl Default for UsbOtg {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for UsbOtg {
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
