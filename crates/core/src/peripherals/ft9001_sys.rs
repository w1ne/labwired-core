// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! FocalTech FT9001 system blocks Renode models in
//! `platforms/cpus/focaltech_ft9001.repl`: the clock and power manager
//! (CPM, 0x4000_4000), the reset controller (0x4000_2000) and the true
//! random number generator (TRNG, 0x4003_B000).
//!
//! No public reference manual exists. Register names and bit positions are
//! from the FocalTech HAL in `focaltech-3545/zephyr-ft9001-public` (commit
//! f982a55e: `cpm_reg.h`, `cpm_drv.c`, `reset_reg.h`, `trng_reg.h`,
//! `trng_drv.c`, `drivers/hwinfo/hwinfo_ft.c`). Renode's
//! `FocalTechFT9001_CPM.cs`, `FocalTechFT9001_Reset.cs` and
//! `FocalTechFT9001_TRNG.cs` were read as a second source.
//!
//! CPM ([`Ft9001Cpm`]): the 0x00..0xE4 word registers are stored as written
//! (reset value 0; no source publishes reset values). The stored dividers,
//! gates, trims and sleep settings have no effect on other models. Two
//! status fields are computed:
//! - OCSR (0x20) bits 8..15, the oscillator STABLE flags, read 1 (as
//!   Renode). The CLK_EN bits 0..7 are stored.
//! - CSWCFGR (0x24) bits 8..11 report the selected SoC clock source:
//!   `1 << (8 + SOC_CLK_SOURCE)`. The HAL selects OSC320M with source 1 and
//!   waits for bit 9; the other codes (0 OSC8M, 2 USBPHY240M, 3 OSCEXT)
//!   follow the bit order of the `*_SELECT` defines and are an inference.
//!   SOC_CLK_SOURCE resets to 1 (OSC320M): the profile runs the core at
//!   160 MHz, and Renode reports OSC320M selected unconditionally.
//!
//! Reset ([`Ft9001Reset`]): RCR (0x00) bit 31 SOFTRST requests a system
//! reset; the other RCR bits are stored without effect (no voltage or
//! clock-fail detectors are modelled). LVDCR/HVDCR/RTR (0x04..0x06) are
//! stored bytes. RSR (0x07) is the reset-cause byte: bit 3 POR after power
//! on (both sources), bit 5 after a software reset and bit 4 after a
//! watchdog reset, as Zephyr's `hwinfo_ft.c` decodes it. `reset_reg.h`
//! names `FT_RSTCAUSE_SOFTWARE` bit 1, but its constants run to bit 16 and
//! cannot be the RSR byte; the hwinfo driver is followed. Renode clears RSR
//! on a software reset instead. RSR writes are ignored (hwinfo's
//! `clear_reset_cause` returns -ENOSYS; no clear mechanism is documented).
//!
//! TRNG ([`Ft9001Trng`]): CTRL (0x00) stores CLK_DIV, EN, EN_IT, EN_ANA and
//! RST_ANA; CLR_IT is write-only; IT (bit 11) always reads 1, as Renode, so
//! `DRV_GetRandomWord` never waits. DATA (0x04) returns the next word of a
//! fixed-seed xorshift32 on every read, so runs are reproducible; the words
//! are not entropy. The interrupt, the SM3 post-processor and the
//! oscillator trim registers (0x08..0x74) are not modelled: they read 0
//! and ignore writes.

use crate::{Peripheral, PeripheralResetCause, SimResult};
use std::any::Any;
use std::cell::Cell;

// ── CPM ──────────────────────────────────────────────────────────────────

const CPM_WORDS: usize = 0xE8 / 4;
const CPM_OCSR: usize = 0x20 / 4;
const CPM_CSWCFGR: usize = 0x24 / 4;
const OCSR_STABLE: u32 = 0xFF << 8;
const CSWCFGR_SRC_MASK: u32 = 0b11;
const CSWCFGR_SELECT_MASK: u32 = 0xF << 8;
const CSWCFGR_SRC_OSC320M: u32 = 1;

/// FT9001 clock and power manager.
#[derive(Debug, Clone)]
pub struct Ft9001Cpm {
    regs: [u32; CPM_WORDS],
}

impl Default for Ft9001Cpm {
    fn default() -> Self {
        Self::new()
    }
}

impl Ft9001Cpm {
    pub fn new() -> Self {
        let mut regs = [0; CPM_WORDS];
        regs[CPM_CSWCFGR] = CSWCFGR_SRC_OSC320M;
        Self { regs }
    }

    fn read_word(&self, index: usize) -> u32 {
        let Some(&stored) = self.regs.get(index) else {
            return 0;
        };
        match index {
            CPM_OCSR => stored | OCSR_STABLE,
            CPM_CSWCFGR => {
                (stored & !CSWCFGR_SELECT_MASK) | (1 << (8 + (stored & CSWCFGR_SRC_MASK)))
            }
            _ => stored,
        }
    }

    fn write_word(&mut self, index: usize, value: u32) {
        let Some(slot) = self.regs.get_mut(index) else {
            return;
        };
        *slot = match index {
            CPM_OCSR => value & !OCSR_STABLE,
            CPM_CSWCFGR => value & !CSWCFGR_SELECT_MASK,
            _ => value,
        };
    }
}

impl Peripheral for Ft9001Cpm {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word((offset / 4) as usize) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let index = (offset / 4) as usize;
        let shift = (offset & 3) * 8;
        let merged = (self.read_word(index) & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_word(index, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word((offset / 4) as usize))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word((offset / 4) as usize, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn on_system_reset(&mut self) {
        *self = Self::new();
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "ft9001_cpm",
            "ocsr": self.read_word(CPM_OCSR),
            "cswcfgr": self.read_word(CPM_CSWCFGR),
        })
    }
}

// ── Reset ────────────────────────────────────────────────────────────────

const RST_RSR: u64 = 0x07;
const RCR_SOFTRST: u32 = 1 << 31;
pub const RSR_POR: u8 = 1 << 3;
pub const RSR_WATCHDOG: u8 = 1 << 4;
pub const RSR_SOFTWARE: u8 = 1 << 5;

/// FT9001 reset controller.
#[derive(Debug, Clone)]
pub struct Ft9001Reset {
    /// RCR at bytes 0..3 (SOFTRST never stored), LVDCR, HVDCR, RTR, RSR.
    bytes: [u8; 8],
    reset_latched: bool,
}

impl Default for Ft9001Reset {
    fn default() -> Self {
        Self::new()
    }
}

impl Ft9001Reset {
    pub fn new() -> Self {
        let mut bytes = [0; 8];
        bytes[RST_RSR as usize] = RSR_POR;
        Self {
            bytes,
            reset_latched: false,
        }
    }

    pub fn rsr(&self) -> u8 {
        self.bytes[RST_RSR as usize]
    }
}

impl Peripheral for Ft9001Reset {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.bytes.get(offset as usize).copied().unwrap_or(0))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            // RCR byte 3 carries SOFTRST (bit 31): write-only, latches a reset.
            3 => {
                if (value as u32) << 24 & RCR_SOFTRST != 0 {
                    self.reset_latched = true;
                }
                self.bytes[3] = value & !((RCR_SOFTRST >> 24) as u8);
            }
            RST_RSR => {}
            0..=6 => self.bytes[offset as usize] = value,
            _ => {}
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn reset_request_capable(&self) -> bool {
        true
    }

    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        self.reset_latched.then_some(PeripheralResetCause::System)
    }

    fn take_reset_request(&mut self) -> bool {
        std::mem::take(&mut self.reset_latched)
    }

    fn on_peripheral_reset(&mut self, cause: PeripheralResetCause) {
        self.bytes[RST_RSR as usize] = match cause {
            PeripheralResetCause::Watchdog => RSR_WATCHDOG,
            PeripheralResetCause::System => RSR_SOFTWARE,
        };
    }

    /// RSR keeps the cause recorded by [`Self::on_peripheral_reset`].
    fn on_system_reset(&mut self) {
        let rsr = self.rsr();
        *self = Self::new();
        self.bytes[RST_RSR as usize] = rsr;
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "ft9001_reset",
            "rcr": u32::from_le_bytes([self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3]]),
            "rsr": self.rsr(),
        })
    }
}

// ── TRNG ─────────────────────────────────────────────────────────────────

const TRNG_CTRL: u64 = 0x00;
const TRNG_DATA: u64 = 0x04;
/// CLK_DIV 0..7, EN 8, EN_IT 9, EN_ANA 16..19, RST_ANA 20..23.
const CTRL_STORED: u32 = 0x00FF_03FF;
const CTRL_IT: u32 = 1 << 11;
const TRNG_SEED: u32 = 0x2545_F491;

/// FT9001 true random number generator (deterministic stand-in).
#[derive(Debug)]
pub struct Ft9001Trng {
    ctrl: u32,
    state: Cell<u32>,
}

impl Default for Ft9001Trng {
    fn default() -> Self {
        Self::new()
    }
}

impl Ft9001Trng {
    pub fn new() -> Self {
        Self {
            ctrl: 0,
            state: Cell::new(TRNG_SEED),
        }
    }

    fn next_word(&self) -> u32 {
        // xorshift32: never reaches the zero state from a non-zero seed.
        let mut x = self.state.get();
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state.set(x);
        x
    }

    fn write_ctrl(&mut self, value: u32) {
        self.ctrl = value & CTRL_STORED;
    }
}

impl Peripheral for Ft9001Trng {
    // A byte read of DATA goes through `read_u32`, so each byte pops a word.
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok((word >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset & !3 == TRNG_CTRL {
            let shift = (offset & 3) * 8;
            self.write_ctrl((self.ctrl & !(0xFF << shift)) | ((value as u32) << shift));
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            TRNG_CTRL => self.ctrl | CTRL_IT,
            TRNG_DATA => self.next_word(),
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset == TRNG_CTRL {
            self.write_ctrl(value);
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word = match offset & !3 {
            TRNG_CTRL => self.ctrl | CTRL_IT,
            _ => 0,
        };
        Some((word >> ((offset & 3) * 8)) as u8)
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn on_system_reset(&mut self) {
        // The generator keeps running: a reboot must not replay the words.
        self.ctrl = 0;
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "ft9001_trng",
            "ctrl": self.ctrl | CTRL_IT,
            "state": self.state.get(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpm_reports_stable_oscillators_and_selected_source() {
        let mut cpm = Ft9001Cpm::new();
        assert_eq!(cpm.read_u32(0x20).unwrap(), 0xFF00);
        assert_eq!(cpm.read_u32(0x24).unwrap(), (1 << 9) | 1);
        // The HAL's OSC320M switch: CSWCFGR |= (CSWCFGR & ~3) | 1.
        cpm.write_u32(0x24, 0).unwrap();
        assert_eq!(cpm.read_u32(0x24).unwrap(), 1 << 8);
        let v = cpm.read_u32(0x24).unwrap();
        cpm.write_u32(0x24, v | ((v & !3) | 1)).unwrap();
        assert_eq!(cpm.read_u32(0x24).unwrap() & 0xF03, (1 << 9) | 1);
        cpm.write_u32(0x24, 3 | (0xF << 8) | (2 << 24)).unwrap();
        assert_eq!(cpm.read_u32(0x24).unwrap(), 3 | (1 << 11) | (2 << 24));
        // STABLE bits are not writable; CLK_EN bits are stored.
        cpm.write_u32(0x20, 0x0008).unwrap();
        assert_eq!(cpm.read_u32(0x20).unwrap(), 0xFF08);
    }

    #[test]
    fn cpm_stores_plain_registers_and_byte_lanes() {
        let mut cpm = Ft9001Cpm::new();
        cpm.write_u32(0x1C, 0x0000_0001).unwrap();
        cpm.write(0x09, 0x5A).unwrap();
        assert_eq!(cpm.read_u32(0x1C).unwrap(), 1);
        assert_eq!(cpm.read_u32(0x08).unwrap(), 0x5A00);
        cpm.write_u32(0xE4, 0xDEAD_BEEF).unwrap();
        assert_eq!(cpm.read_u32(0xE4).unwrap(), 0xDEAD_BEEF);
        // Past the register file: reads 0, writes ignored.
        cpm.write_u32(0xE8, 1).unwrap();
        assert_eq!(cpm.read_u32(0xE8).unwrap(), 0);
        cpm.on_system_reset();
        assert_eq!(cpm.read_u32(0x08).unwrap(), 0);
        assert_eq!(cpm.read_u32(0x24).unwrap(), (1 << 9) | 1);
    }

    #[test]
    fn reset_por_then_softrst_records_software_cause() {
        let mut rst = Ft9001Reset::new();
        assert_eq!(rst.read(0x07).unwrap(), RSR_POR);
        assert!(!rst.take_reset_request());
        // RSR ignores writes.
        rst.write(0x07, 0).unwrap();
        assert_eq!(rst.rsr(), RSR_POR);
        // Stored RCR bits and the detector bytes read back.
        rst.write_u32(0x00, 0x0000_0101).unwrap();
        rst.write(0x04, 0x12).unwrap();
        assert_eq!(rst.read_u32(0x00).unwrap(), 0x0000_0101);
        assert_eq!(rst.read_u32(0x04).unwrap(), (RSR_POR as u32) << 24 | 0x12);
        assert!(!rst.take_reset_request());

        rst.write_u32(0x00, RCR_SOFTRST | 0x0101).unwrap();
        assert_eq!(rst.read_u32(0x00).unwrap() & RCR_SOFTRST, 0);
        assert_eq!(
            rst.latched_reset_cause(),
            Some(PeripheralResetCause::System)
        );
        assert!(rst.take_reset_request());
        assert!(!rst.take_reset_request());
        rst.on_peripheral_reset(PeripheralResetCause::System);
        rst.on_system_reset();
        assert_eq!(rst.rsr(), RSR_SOFTWARE);
        assert_eq!(rst.read_u32(0x00).unwrap(), 0);
        rst.on_peripheral_reset(PeripheralResetCause::Watchdog);
        assert_eq!(rst.rsr(), RSR_WATCHDOG);
        rst.on_node_reset();
        assert_eq!(rst.rsr(), RSR_POR);
    }

    #[test]
    fn trng_ctrl_fields_and_reproducible_words() {
        let mut t = Ft9001Trng::new();
        assert_eq!(t.read_u32(0x00).unwrap(), CTRL_IT);
        // HAL init: prescaler 59, RST_ANA, EN_ANA, CLR_IT, EN.
        t.write_u32(0x00, 59 | (0xF << 20) | (0xF << 16) | (1 << 10) | (1 << 8))
            .unwrap();
        assert_eq!(
            t.read_u32(0x00).unwrap(),
            59 | (0xF << 20) | (0xF << 16) | (1 << 8) | CTRL_IT
        );
        let a = t.read_u32(0x04).unwrap();
        let b = t.read_u32(0x04).unwrap();
        assert_ne!(a, b);
        let fresh = Ft9001Trng::new();
        assert_eq!(fresh.read_u32(0x04).unwrap(), a);
        assert_eq!(fresh.read_u32(0x04).unwrap(), b);
        // Unmodelled registers read 0 and ignore writes.
        t.write_u32(0x70, 0xFFFF_FFFF).unwrap();
        assert_eq!(t.read_u32(0x70).unwrap(), 0);
        // A system reset clears CTRL but does not replay the sequence.
        t.on_system_reset();
        assert_eq!(t.read_u32(0x00).unwrap(), CTRL_IT);
        assert_ne!(t.read_u32(0x04).unwrap(), a);
    }
}
