// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Renesas RX SYSTEM block (clock generation, module stop, write
//! protection) as a register file with the status bits the RX Driver
//! Package's `mcu_clock_setup` polls derived from their control bits.
//!
//! RX261 (RX261 Group User's Manual: Hardware, R01UH1045, "Clock Generation
//! Circuit"; offsets checked against the RDP `iodefine.h` `struct st_system`
//! by compiling `offsetof` with the RX GCC), window 0x00080000-0x000803FF:
//!
//! - **OSCOVFSR** (+0x3C, read-only): MOOVF (bit0) = !MOSCCR.MOSTP (+0x32),
//!   PLOVF (bit2) = !PLLCR2.PLLEN (+0x2A), HCOVF (bit3) = !HOCOCR.HCSTP
//!   (+0x36), PL2OVF (bit5) = !PLL2CR2.PLL2EN (+0x2E). Oscillators settle at
//!   once; MOSCWTCR and the other wait-time registers are stored only.
//! - **OPCCR** (+0xA0): OPCMTSF (bit4, transition in progress) always reads
//!   0, so an operating-power-mode change completes immediately.
//! - **USBCKCR** (+0x74) / **CANFDCKCR** (+0x76): CKSRDY (bit7) reads back
//!   CKSREQ (bit6), the clock-switch handshake.
//!
//! Reset values: MOSCCR, HOCOCR, PLLCR2 and PLL2CR2 reset to "stopped"
//! (0x01) and SCKCR3 to LOCO (0x0000), the RX261 values `mcu_clock_setup`
//! starts from. Everything else resets to 0 and every register reads back
//! what was written. No clock rate is derived (`cpu_hz` is the chip's
//! nominal ICLK), and the PRCR protect key is not enforced.

use crate::{Peripheral, SimResult};

const SIZE: usize = 0x400;
const PLLCR2: usize = 0x2A;
const PLL2CR2: usize = 0x2E;
const MOSCCR: usize = 0x32;
const HOCOCR: usize = 0x36;
const OSCOVFSR: usize = 0x3C;
const USBCKCR: usize = 0x74;
const CANFDCKCR: usize = 0x76;
const OPCCR: usize = 0xA0;

const MOOVF: u8 = 1 << 0;
const PLOVF: u8 = 1 << 2;
const HCOVF: u8 = 1 << 3;
const PL2OVF: u8 = 1 << 5;
const OPCMTSF: u8 = 1 << 4;
const CKSREQ: u8 = 1 << 6;
const CKSRDY: u8 = 1 << 7;

#[derive(Debug)]
pub struct RxSysc {
    regs: Box<[u8]>,
}

impl Default for RxSysc {
    fn default() -> Self {
        Self::new()
    }
}

impl RxSysc {
    pub fn new() -> Self {
        let mut regs = vec![0u8; SIZE].into_boxed_slice();
        for off in [MOSCCR, HOCOCR, PLLCR2, PLL2CR2] {
            regs[off] = 0x01;
        }
        Self { regs }
    }

    fn oscovfsr(&self) -> u8 {
        let running = |off: usize| self.regs[off] & 1 == 0;
        let mut v = 0;
        if running(MOSCCR) {
            v |= MOOVF;
        }
        if running(PLLCR2) {
            v |= PLOVF;
        }
        if running(HOCOCR) {
            v |= HCOVF;
        }
        if running(PLL2CR2) {
            v |= PL2OVF;
        }
        v
    }
}

impl Peripheral for RxSysc {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let off = offset as usize;
        if off >= SIZE {
            return Ok(0);
        }
        let v = self.regs[off];
        Ok(match off {
            OSCOVFSR => self.oscovfsr(),
            OPCCR => v & !OPCMTSF,
            USBCKCR | CANFDCKCR => (v & !CKSRDY) | if v & CKSREQ != 0 { CKSRDY } else { 0 },
            _ => v,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset as usize;
        if off < SIZE && off != OSCOVFSR {
            self.regs[off] = value;
        }
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        let nonzero: serde_json::Map<String, serde_json::Value> = self
            .regs
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0)
            .map(|(i, v)| (format!("{i:#05x}"), serde_json::json!(v)))
            .collect();
        serde_json::json!({
            "peripheral": "rx_sysc",
            "oscovfsr": self.oscovfsr(),
            "regs": nonzero,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oscovfsr_follows_the_stop_bits() {
        let mut s = RxSysc::new();
        assert_eq!(s.read(OSCOVFSR as u64).unwrap(), 0);
        s.write(MOSCCR as u64, 0).unwrap();
        assert_eq!(s.read(OSCOVFSR as u64).unwrap(), MOOVF);
        s.write(PLLCR2 as u64, 0).unwrap();
        s.write(HOCOCR as u64, 0).unwrap();
        s.write(PLL2CR2 as u64, 0).unwrap();
        assert_eq!(
            s.read(OSCOVFSR as u64).unwrap(),
            MOOVF | PLOVF | HCOVF | PL2OVF
        );
        // Stopping the PLL again drops PLOVF (mcu_clock_setup waits for it).
        s.write(PLLCR2 as u64, 1).unwrap();
        assert_eq!(s.read(OSCOVFSR as u64).unwrap() & PLOVF, 0);
        // The status register itself is read-only.
        s.write(OSCOVFSR as u64, 0xFF).unwrap();
        assert_eq!(s.read(OSCOVFSR as u64).unwrap() & PLOVF, 0);
    }

    #[test]
    fn clock_switch_and_power_mode_handshakes_complete() {
        let mut s = RxSysc::new();
        s.write(USBCKCR as u64, CKSREQ | 2).unwrap();
        assert_eq!(s.read(USBCKCR as u64).unwrap(), CKSREQ | CKSRDY | 2);
        s.write(USBCKCR as u64, 2).unwrap();
        assert_eq!(s.read(USBCKCR as u64).unwrap(), 2);
        s.write(OPCCR as u64, 0x06).unwrap();
        assert_eq!(s.read(OPCCR as u64).unwrap() & OPCMTSF, 0);
        assert_eq!(s.read(OPCCR as u64).unwrap(), 0x06);
    }
}

/// A Renesas RX register block held as read-back storage.
///
/// - `rx_flash`: the FLASH control registers (RX261 0x007FC000-0x007FFFFF)
///   with the RX protection-key convention: a 16-bit write whose upper byte
///   is the key 0xAA (FENTRYR, MEMWAITR, ...) stores only the low byte and
///   the upper byte reads 0. Other writes are stored as is. No
///   programming/erase sequencer (`FSTATR`, the FACI command area) is
///   modelled; `mcu_clock_setup` only programs MEMWAITR and reads it back.
/// - `rx_rtc`: the RTC registers as plain storage (no counting). The RDP
///   sub-clock setup writes RCR1/RCR2 and waits to read START/CNTMD/RESET
///   back.
#[derive(Debug)]
pub struct RxFlashRegs {
    name: &'static str,
    keyed: bool,
    regs: Box<[u8]>,
}

const FLASH_REGS_SIZE: usize = 0x4000;
const RTC_REGS_SIZE: usize = 0x100;
const FLASH_KEY: u16 = 0xAA00;

impl Default for RxFlashRegs {
    fn default() -> Self {
        Self::new()
    }
}

impl RxFlashRegs {
    pub fn new() -> Self {
        Self {
            name: "rx_flash",
            keyed: true,
            regs: vec![0u8; FLASH_REGS_SIZE].into_boxed_slice(),
        }
    }

    pub fn rtc() -> Self {
        Self {
            name: "rx_rtc",
            keyed: false,
            regs: vec![0u8; RTC_REGS_SIZE].into_boxed_slice(),
        }
    }
}

impl Peripheral for RxFlashRegs {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.regs.get(offset as usize).copied().unwrap_or(0))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if let Some(r) = self.regs.get_mut(offset as usize) {
            *r = value;
        }
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if self.keyed && value & 0xFF00 == FLASH_KEY {
            self.write(offset, value as u8)?;
            return self.write(offset + 1, 0);
        }
        self.write(offset, value as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
    }

    fn snapshot(&self) -> serde_json::Value {
        let nonzero: serde_json::Map<String, serde_json::Value> = self
            .regs
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0)
            .map(|(i, v)| (format!("{i:#06x}"), serde_json::json!(v)))
            .collect();
        serde_json::json!({ "peripheral": self.name, "regs": nonzero })
    }
}

#[cfg(test)]
mod flash_tests {
    use super::*;

    #[test]
    fn keyed_word_write_stores_the_low_byte_only() {
        let mut f = RxFlashRegs::new();
        // MEMWAITR (+0x3FC0): RDP writes 0xAA01 and polls for 0x0001.
        f.write_u16(0x3FC0, 0xAA01).unwrap();
        assert_eq!(f.read_u16(0x3FC0).unwrap(), 0x0001);
        f.write_u16(0x3FC0, 0x1234).unwrap();
        assert_eq!(f.read_u16(0x3FC0).unwrap(), 0x1234);
    }

    #[test]
    fn rtc_storage_has_no_key() {
        let mut r = RxFlashRegs::rtc();
        r.write_u16(0x24, 0xAA01).unwrap();
        assert_eq!(r.read_u16(0x24).unwrap(), 0xAA01);
    }
}
