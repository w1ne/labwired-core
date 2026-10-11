// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32F4 system configuration controller (SYSCFG, RM0090 §9 / RM0368 §7).
//!
//! What matters to the rest of the model is `SYSCFG_EXTICR1..4`: on the F4
//! the EXTI block has no port mux of its own, and these four registers pick
//! which GPIO port drives each of EXTI lines 0..15 (the F1 keeps the same
//! fields in `AFIO_EXTICRx`). The bus asks this model through
//! [`Peripheral::exti_line_source`] when an external GPIO edge arrives, so
//! the EXTI raises the interrupt only for the selected port.
//!
//! `MEMRMP` and `PMC` are plain read/write fields; `CMPCR.READY` follows
//! `CMPCR.CMP_PD` at once (the compensation cell has no settling time here),
//! so HAL's `HAL_EnableCompensationCell` poll completes.

use crate::{Peripheral, SimResult};
use std::any::Any;

/// STM32F4 SYSCFG register file.
#[derive(Debug, Default, serde::Serialize)]
pub struct Stm32F4Syscfg {
    memrmp: u32,
    pmc: u32,
    exticr: [u32; 4],
    cmpcr: u32,
}

impl Stm32F4Syscfg {
    pub fn new() -> Self {
        Self::default()
    }

    /// The GPIO port index (0 = A) `SYSCFG_EXTICRx` routes to EXTI `line`.
    pub fn exti_port(&self, line: u8) -> Option<u8> {
        if line >= 16 {
            return None;
        }
        let word = self.exticr[usize::from(line / 4)];
        Some(((word >> (u32::from(line % 4) * 4)) & 0xF) as u8)
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.memrmp,
            0x04 => self.pmc,
            0x08 => self.exticr[0],
            0x0C => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            // READY (bit 8) mirrors CMP_PD (bit 0): ready as soon as enabled.
            0x20 => self.cmpcr | ((self.cmpcr & 1) << 8),
            _ => {
                crate::census_reg!("syscfg:Stm32F4Syscfg", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // MEM_MODE[1:0].
            0x00 => self.memrmp = value & 0x3,
            // ADC1DC2 (bit 16, F401/F411) and MII_RMII_SEL (bit 23, F407).
            0x04 => self.pmc = value & ((1 << 16) | (1 << 23)),
            // EXTICRx: four 4-bit port selects in [15:0], [31:16] reserved.
            0x08 => self.exticr[0] = value & 0xFFFF,
            0x0C => self.exticr[1] = value & 0xFFFF,
            0x10 => self.exticr[2] = value & 0xFFFF,
            0x14 => self.exticr[3] = value & 0xFFFF,
            // CMP_PD (bit 0); READY is read-only.
            0x20 => self.cmpcr = value & 1,
            _ => {
                crate::census_reg!("syscfg:Stm32F4Syscfg", offset, "write");
            }
        }
    }
}

impl Peripheral for Stm32F4Syscfg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = self.read_reg(offset & !3);
        Ok(((reg >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let shift = (offset % 4) * 8;
        let mut reg = self.read_reg(reg_offset) & !(1 << 8);
        reg &= !(0xFF << shift);
        reg |= u32::from(value) << shift;
        self.write_reg(reg_offset, reg);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn exti_line_source(&self, line: u8) -> Option<u8> {
        self.exti_port(line)
    }

    /// A configuration register bank: no `tick`, nothing to walk.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// STM32L0 SYSCFG (RM0367 §10; the COMP CSRs share the window): the same
/// `SYSCFG_EXTICR1..4` line-source mux as the F4, at the same offsets
/// (0x08..0x14), around L0-specific configuration words.
///
/// `CFGR1`: MEM_MODE (1:0) and UFB (3) stick; BOOT_MODE (9:8) reads 00, main
/// Flash, the only boot this model has. `CFGR2` and the two `COMPx_CSR` are
/// stored (no comparator is modelled: COMP1VALUE / COMP2VALUE read 0, and a
/// set LOCK bit freezes the word as on silicon). `CFGR3` stores its enables;
/// each reference or buffer reports ready (the RDYF bits 26..30) as soon as
/// its enable is set, since no start-up time is modelled.
#[derive(Debug, Default, serde::Serialize)]
pub struct Stm32L0Syscfg {
    cfgr1: u32,
    cfgr2: u32,
    exticr: [u32; 4],
    comp1_csr: u32,
    comp2_csr: u32,
    cfgr3: u32,
}

impl Stm32L0Syscfg {
    pub fn new() -> Self {
        Self::default()
    }

    /// The GPIO port index (0 = A) `SYSCFG_EXTICRx` routes to EXTI `line`.
    pub fn exti_port(&self, line: u8) -> Option<u8> {
        if line >= 16 {
            return None;
        }
        let word = self.exticr[usize::from(line / 4)];
        Some(((word >> (u32::from(line % 4) * 4)) & 0xF) as u8)
    }

    /// CFGR3 with the ready flags its enables imply: VREFINT_RDYF (30) from
    /// EN_BGAP (0), VREFINT_COMP_RDYF (29) from ENBUF_VREFINT_COMP (12),
    /// VREFINT_ADC_RDYF (28) from ENBUF_BGAP_ADC (8), SENSOR_ADC_RDYF (27)
    /// from ENBUF_SENSOR_ADC (9), REF_RC48MHz_RDYF (26) from ENREF_RC48MHz
    /// (13).
    fn cfgr3_view(&self) -> u32 {
        let on = |bit: u32| (self.cfgr3 >> bit) & 1;
        self.cfgr3 | (on(0) << 30) | (on(12) << 29) | (on(8) << 28) | (on(9) << 27) | (on(13) << 26)
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cfgr1,
            0x04 => self.cfgr2,
            0x08 => self.exticr[0],
            0x0C => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            0x18 => self.comp1_csr,
            0x1C => self.comp2_csr,
            0x20 => self.cfgr3_view(),
            _ => {
                crate::census_reg!("syscfg:Stm32L0Syscfg", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        // A set LOCK bit (31) makes the word read-only until reset.
        let locked = |word: u32| word & (1 << 31) != 0;
        match offset {
            // MEM_MODE[1:0], UFB[3]; BOOT_MODE is read-only.
            0x00 => self.cfgr1 = value & 0b1011,
            // FWDISEN (0), I2C_PB6..9_FMP / I2C1_FMP / I2C2_FMP (13:8).
            0x04 => self.cfgr2 = value & 0x3F01,
            0x08 => self.exticr[0] = value & 0xFFFF,
            0x0C => self.exticr[1] = value & 0xFFFF,
            0x10 => self.exticr[2] = value & 0xFFFF,
            0x14 => self.exticr[3] = value & 0xFFFF,
            // COMP1VALUE (30) is read-only.
            0x18 if !locked(self.comp1_csr) => self.comp1_csr = value & !(1 << 30),
            // COMP2VALUE (20) is read-only.
            0x1C if !locked(self.comp2_csr) => self.comp2_csr = value & !(1 << 20),
            0x18 | 0x1C => {}
            // The RDYF bits [30:26] are read-only.
            0x20 if !locked(self.cfgr3) => self.cfgr3 = value & !(0x1F << 26),
            0x20 => {}
            _ => {
                crate::census_reg!("syscfg:Stm32L0Syscfg", offset, "write");
            }
        }
    }
}

impl Peripheral for Stm32L0Syscfg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = self.read_reg(offset & !3);
        Ok(((reg >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let shift = (offset % 4) * 8;
        let mut reg = self.read_reg(reg_offset);
        reg &= !(0xFF << shift);
        reg |= u32::from(value) << shift;
        self.write_reg(reg_offset, reg);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn exti_line_source(&self, line: u8) -> Option<u8> {
        self.exti_port(line)
    }

    /// A configuration register bank: no `tick`, nothing to walk.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod l0_tests {
    use super::Stm32L0Syscfg;
    use crate::Peripheral;

    #[test]
    fn exticr_selects_the_port_per_line() {
        let mut s = Stm32L0Syscfg::new();
        assert_eq!(s.exti_line_source(0), Some(0), "reset: port A");
        // EXTICR4: line 13 -> port C (2), the Nucleo-L073RZ user button.
        s.write_u32(0x14, 2 << 4).unwrap();
        assert_eq!(s.exti_line_source(13), Some(2));
        assert_eq!(s.exti_line_source(12), Some(0));
        assert_eq!(s.exti_line_source(16), None);
    }

    #[test]
    fn cfgr3_reports_ready_for_what_is_enabled() {
        let mut s = Stm32L0Syscfg::new();
        assert_eq!(s.read_u32(0x20).unwrap(), 0);
        // EN_BGAP + ENBUF_BGAP_ADC.
        s.write_u32(0x20, (1 << 0) | (1 << 8)).unwrap();
        let v = s.read_u32(0x20).unwrap();
        assert_ne!(v & (1 << 30), 0, "VREFINT_RDYF");
        assert_ne!(v & (1 << 28), 0, "VREFINT_ADC_RDYF");
        assert_eq!(v & (1 << 27), 0, "SENSOR_ADC_RDYF stays clear");
        // A byte store keeps the other enables.
        s.write(0x21, 0x02).unwrap(); // ENBUF_SENSOR_ADC, drops ENBUF_BGAP_ADC
        let v = s.read_u32(0x20).unwrap();
        assert_ne!(v & (1 << 27), 0);
        assert_eq!(v & (1 << 28), 0);
        assert_ne!(v & 1, 0);
    }

    #[test]
    fn a_locked_comparator_word_is_frozen() {
        let mut s = Stm32L0Syscfg::new();
        s.write_u32(0x18, (1 << 31) | 1).unwrap();
        s.write_u32(0x18, 0).unwrap();
        assert_eq!(s.read_u32(0x18).unwrap(), (1 << 31) | 1);
        s.write_u32(0x00, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x00).unwrap(), 0b1011, "BOOT_MODE reads 00");
    }
}

#[cfg(test)]
mod tests {
    use super::Stm32F4Syscfg;
    use crate::Peripheral;

    #[test]
    fn exticr_selects_the_port_per_line() {
        let mut s = Stm32F4Syscfg::new();
        // EXTICR1: line 1 -> port C (2); EXTICR3: line 8 -> port A, line 10 -> port H (7).
        s.write_u32(0x08, 2 << 4).unwrap();
        s.write_u32(0x10, 7 << 8).unwrap();
        assert_eq!(s.exti_line_source(0), Some(0));
        assert_eq!(s.exti_line_source(1), Some(2));
        assert_eq!(s.exti_line_source(8), Some(0));
        assert_eq!(s.exti_line_source(10), Some(7));
        assert_eq!(s.exti_line_source(16), None);
        // Reserved upper half reads 0.
        s.write_u32(0x14, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x14).unwrap(), 0xFFFF);
    }

    #[test]
    fn byte_writes_keep_the_other_fields() {
        let mut s = Stm32F4Syscfg::new();
        s.write_u32(0x0C, 0x1234).unwrap();
        s.write(0x0D, 0x05).unwrap();
        assert_eq!(s.read_u32(0x0C).unwrap(), 0x0534);
    }

    #[test]
    fn compensation_cell_is_ready_once_enabled() {
        let mut s = Stm32F4Syscfg::new();
        assert_eq!(s.read_u32(0x20).unwrap() & (1 << 8), 0);
        s.write_u32(0x20, 1).unwrap();
        assert_eq!(s.read_u32(0x20).unwrap(), 0x101);
    }
}
