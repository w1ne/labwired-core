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

/// The port index (0 = A, by letter) the 4-bit `EXTICRx` field of `line`
/// selects, through the family's code→port table `ports` (code `n` selects
/// `ports[n]`). A code past the table selects no port — `Some(u8::MAX)`,
/// which matches no port, so the line stays quiet as on silicon. `None` only
/// for lines 16+, which have no mux.
fn exticr_port(exticr: &[u32; 4], line: u8, ports: &[u8]) -> Option<u8> {
    if line >= 16 {
        return None;
    }
    let code = (exticr[usize::from(line / 4)] >> (u32::from(line % 4) * 4)) & 0xF;
    Some(ports.get(code as usize).copied().unwrap_or(u8::MAX))
}

/// STM32F4 SYSCFG register file.
#[derive(Debug, Default, serde::Serialize)]
pub struct Stm32F4Syscfg {
    memrmp: u32,
    pmc: u32,
    exticr: [u32; 4],
    cmpcr: u32,
    /// The F74x/F75x field set (`profile: stm32f7`) instead of the F4 one.
    #[serde(skip)]
    f7: bool,
}

/// F4 `MEMRMP`: MEM_MODE[1:0].
const F4_MEMRMP_MASK: u32 = 0x3;
/// F4 `PMC`: ADC1DC2 (bit 16, F401/F411) and MII_RMII_SEL (bit 23, F407).
const F4_PMC_MASK: u32 = (1 << 16) | (1 << 23);
/// F74x `MEMRMP` (RM0385 §7.2.1, `stm32f746xx.h`): SWP_FMC[11:10]. MEM_BOOT
/// (bit 0) is read-only and reads 0, the BOOT_ADD0 boot this model has.
const F7_MEMRMP_MASK: u32 = 0x3 << 10;
/// F74x `PMC` (RM0385 §7.2.2): I2C1..4_FMP and PB6..PB9_FMP (bits 0..7),
/// ADC1..3DC2 (bits 16..18), MII_RMII_SEL (bit 23).
const F7_PMC_MASK: u32 = 0xFF | (0x7 << 16) | (1 << 23);

impl Stm32F4Syscfg {
    pub fn new() -> Self {
        Self::default()
    }

    /// The STM32F74x/F75x SYSCFG (RM0385 §7): the F4 EXTICR offsets and the
    /// by-letter port codes (A = 0 .. K = 10), CMPCR at 0x20, and F7 fields in
    /// `MEMRMP` and `PMC`.
    pub fn new_f7() -> Self {
        Self {
            f7: true,
            ..Self::default()
        }
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
            0x00 => {
                self.memrmp = value
                    & if self.f7 {
                        F7_MEMRMP_MASK
                    } else {
                        F4_MEMRMP_MASK
                    }
            }
            0x04 => self.pmc = value & if self.f7 { F7_PMC_MASK } else { F4_PMC_MASK },
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
    /// The L0 numbers its ports A..E = 0..4 and H = 5 (`stm32l073xx.h`
    /// `SYSCFG_EXTICR1_EXTI0_PH = 5`); the other codes select no port.
    pub fn exti_port(&self, line: u8) -> Option<u8> {
        exticr_port(&self.exticr, line, &[0, 1, 2, 3, 4, 7])
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

/// STM32L1 SYSCFG (RM0038 §8, `stm32l151xd.h` SYSCFG_TypeDef): MEMRMP @0x00,
/// PMC @0x04 and the `SYSCFG_EXTICR1..4` line-source mux at 0x08..0x14 — the
/// F4 offsets, but NOT the F4 port numbering. The L1 codes are A..E = 0..4,
/// **H = 5, F = 6, G = 7** (`SYSCFG_EXTICR1_EXTI0_PH = 5`, `_PF = 6`,
/// `_PG = 7`): port H came before F/G on the first L1 parts and kept its
/// code. Routing L1 EXTICR through the F4 model would send a PH edge to the
/// line only when firmware asked for PG.
///
/// `MEMRMP`: MEM_MODE (1:0) sticks; BOOT_MODE (9:8) reads 00, main Flash, the
/// only boot this model has. `PMC`: USB_PU (0) — the only PMC bit the
/// STM32L151xD header defines — sticks.
#[derive(Debug, Default, serde::Serialize)]
pub struct Stm32L1Syscfg {
    memrmp: u32,
    pmc: u32,
    exticr: [u32; 4],
}

impl Stm32L1Syscfg {
    pub fn new() -> Self {
        Self::default()
    }

    /// The GPIO port index (0 = A, by letter) `SYSCFG_EXTICRx` routes to
    /// EXTI `line`.
    pub fn exti_port(&self, line: u8) -> Option<u8> {
        exticr_port(&self.exticr, line, &[0, 1, 2, 3, 4, 7, 5, 6])
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.memrmp,
            0x04 => self.pmc,
            0x08 => self.exticr[0],
            0x0C => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            _ => {
                crate::census_reg!("syscfg:Stm32L1Syscfg", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // MEM_MODE[1:0]; BOOT_MODE is read-only.
            0x00 => self.memrmp = value & 0x3,
            0x04 => self.pmc = value & 0x1,
            0x08 => self.exticr[0] = value & 0xFFFF,
            0x0C => self.exticr[1] = value & 0xFFFF,
            0x10 => self.exticr[2] = value & 0xFFFF,
            0x14 => self.exticr[3] = value & 0xFFFF,
            _ => {
                crate::census_reg!("syscfg:Stm32L1Syscfg", offset, "write");
            }
        }
    }
}

impl Peripheral for Stm32L1Syscfg {
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
mod l1_tests {
    use super::Stm32L1Syscfg;
    use crate::Peripheral;

    #[test]
    fn exticr_codes_follow_the_l1_port_numbering() {
        let mut s = Stm32L1Syscfg::new();
        assert_eq!(s.exti_line_source(0), Some(0), "reset: port A");
        // EXTICR1: line 0 -> code 5, line 1 -> code 6, line 2 -> code 7,
        // line 3 -> code 2.
        s.write_u32(0x08, 5 | (6 << 4) | (7 << 8) | (2 << 12))
            .unwrap();
        assert_eq!(s.exti_line_source(0), Some(7), "code 5 is port H");
        assert_eq!(s.exti_line_source(1), Some(5), "code 6 is port F");
        assert_eq!(s.exti_line_source(2), Some(6), "code 7 is port G");
        assert_eq!(s.exti_line_source(3), Some(2), "code 2 is port C");
        // A reserved code selects no port at all.
        s.write_u32(0x14, 0x8 << 4).unwrap();
        assert_eq!(s.exti_line_source(13), Some(u8::MAX));
        assert_eq!(s.exti_line_source(16), None);
    }

    #[test]
    fn registers_keep_only_their_fields() {
        let mut s = Stm32L1Syscfg::new();
        s.write_u32(0x00, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x00).unwrap(), 0x3, "BOOT_MODE reads 00");
        s.write_u32(0x04, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x04).unwrap(), 0x1);
        s.write_u32(0x10, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x10).unwrap(), 0xFFFF);
        // A byte store reaches one field only.
        s.write(0x11, 0x00).unwrap();
        assert_eq!(s.read_u32(0x10).unwrap(), 0x00FF);
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
        // EXTICR1 line 0 -> code 5 is port H (index 7 by letter); code 6 is
        // reserved on the L0 and selects nothing.
        s.write_u32(0x08, 5 | (6 << 4)).unwrap();
        assert_eq!(s.exti_line_source(0), Some(7));
        assert_eq!(s.exti_line_source(1), Some(u8::MAX));
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
    fn f7_profile_keeps_the_f74x_memrmp_and_pmc_fields() {
        let mut f7 = Stm32F4Syscfg::new_f7();
        f7.write_u32(0x00, 0xFFFF_FFFF).unwrap();
        f7.write_u32(0x04, 0xFFFF_FFFF).unwrap();
        // SWP_FMC only; MEM_BOOT reads 0.
        assert_eq!(f7.read_u32(0x00).unwrap(), 0x0C00);
        assert_eq!(f7.read_u32(0x04).unwrap(), 0x0087_00FF);
        let mut f4 = Stm32F4Syscfg::new();
        f4.write_u32(0x00, 0xFFFF_FFFF).unwrap();
        f4.write_u32(0x04, 0xFFFF_FFFF).unwrap();
        assert_eq!(f4.read_u32(0x00).unwrap(), 0x3);
        assert_eq!(f4.read_u32(0x04).unwrap(), 0x0081_0000);
        // The same EXTICR mux, port K = 10.
        f7.write_u32(0x08, 10 << 12).unwrap();
        assert_eq!(f7.exti_line_source(3), Some(10));
    }

    #[test]
    fn compensation_cell_is_ready_once_enabled() {
        let mut s = Stm32F4Syscfg::new();
        assert_eq!(s.read_u32(0x20).unwrap() & (1 << 8), 0);
        s.write_u32(0x20, 1).unwrap();
        assert_eq!(s.read_u32(0x20).unwrap(), 0x101);
    }
}

/// STM32F0 SYSCFG (RM0091 §9, `stm32f072xb.h` SYSCFG_TypeDef): CFGR1 @0x00,
/// a reserved word @0x04, the `SYSCFG_EXTICR1..4` line-source mux at
/// 0x08..0x14 and CFGR2 @0x18. The F0 port codes are by letter, A..F = 0..5
/// (`SYSCFG_EXTICR1_EXTI0_PF = 5`); 6..15 select no port.
///
/// `CFGR1`: MEM_MODE (1:0), the DMA remap bits (14:8, 30:24) and the I2C
/// Fm+ drive bits (21:16) stick; MEM_MODE resets to 00, main Flash, the only
/// boot this model has. The DMA remap bits are register state only: the
/// engine's DMA request table is the reset (unremapped) one. `CFGR2`: the
/// three TIM1-break lock bits (2:0) are set-once until reset, and SRAM_PEF
/// (8) is rc_w1 and never set — this engine has no SRAM parity.
///
/// The F04x CFGR1 ([`Self::new_f04x`], `stm32f042x6.h`) has PA11_PA12_RMP
/// (4) and Fm+ bits up to I2C_FMP_PA10 (23), but none of the F07x DMA remap
/// bits 13:14 / 24:30.
#[derive(Debug, serde::Serialize)]
pub struct Stm32F0Syscfg {
    cfgr1: u32,
    exticr: [u32; 4],
    cfgr2: u32,
    #[serde(skip)]
    cfgr1_mask: u32,
}

impl Default for Stm32F0Syscfg {
    fn default() -> Self {
        Self {
            cfgr1: 0,
            exticr: [0; 4],
            cfgr2: 0,
            cfgr1_mask: Self::CFGR1_MASK,
        }
    }
}

impl Stm32F0Syscfg {
    const CFGR1_MASK: u32 = 0x7F3F_7F03;
    const CFGR1_MASK_F04X: u32 = 0x00FF_1F13;
    const CFGR2_LOCKS: u32 = 0x7;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_f04x() -> Self {
        Self {
            cfgr1_mask: Self::CFGR1_MASK_F04X,
            ..Self::default()
        }
    }

    /// The GPIO port index (0 = A, by letter) `SYSCFG_EXTICRx` routes to
    /// EXTI `line`.
    pub fn exti_port(&self, line: u8) -> Option<u8> {
        exticr_port(&self.exticr, line, &[0, 1, 2, 3, 4, 5])
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cfgr1,
            0x08 => self.exticr[0],
            0x0C => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            0x18 => self.cfgr2,
            _ => {
                crate::census_reg!("syscfg:Stm32F0Syscfg", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cfgr1 = value & self.cfgr1_mask,
            0x08 => self.exticr[0] = value & 0xFFFF,
            0x0C => self.exticr[1] = value & 0xFFFF,
            0x10 => self.exticr[2] = value & 0xFFFF,
            0x14 => self.exticr[3] = value & 0xFFFF,
            // Lock bits only ever go 0 -> 1.
            0x18 => self.cfgr2 |= value & Self::CFGR2_LOCKS,
            _ => {
                crate::census_reg!("syscfg:Stm32F0Syscfg", offset, "write");
            }
        }
    }
}

impl Peripheral for Stm32F0Syscfg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = self.read_reg(offset & !3);
        Ok(((reg >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let shift = (offset % 4) * 8;
        // CFGR2 is set-once: merge only the byte written, never the read-back.
        if reg_offset == 0x18 {
            self.write_reg(reg_offset, u32::from(value) << shift);
            return Ok(());
        }
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
mod f0_tests {
    use super::Stm32F0Syscfg;
    use crate::Peripheral;

    #[test]
    fn exticr_codes_follow_the_f0_port_letters() {
        let mut s = Stm32F0Syscfg::new();
        assert_eq!(s.exti_line_source(0), Some(0), "reset: port A");
        // EXTICR1: line 0 -> code 5 (PF), line 1 -> code 2 (PC),
        // line 2 -> code 6 (reserved).
        s.write_u32(0x08, 5 | (2 << 4) | (6 << 8)).unwrap();
        assert_eq!(s.exti_line_source(0), Some(5), "code 5 is port F");
        assert_eq!(s.exti_line_source(1), Some(2), "code 2 is port C");
        assert_eq!(s.exti_line_source(2), Some(u8::MAX), "code 6 is no port");
        assert_eq!(s.exti_line_source(16), None);
    }

    #[test]
    fn f04x_cfgr1_keeps_the_f04x_fields() {
        let mut s = Stm32F0Syscfg::new_f04x();
        s.write_u32(0x00, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x00).unwrap(), 0x00FF_1F13);
    }

    #[test]
    fn registers_keep_only_their_fields() {
        let mut s = Stm32F0Syscfg::new();
        s.write_u32(0x00, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x00).unwrap(), 0x7F3F_7F03);
        assert_eq!(s.read_u32(0x04).unwrap(), 0, "reserved word");
        s.write_u32(0x10, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x10).unwrap(), 0xFFFF);
        // CFGR2 lock bits are set-once; SRAM_PEF never sets.
        s.write_u32(0x18, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x18).unwrap(), 0x7);
        s.write_u32(0x18, 0).unwrap();
        assert_eq!(s.read_u32(0x18).unwrap(), 0x7, "locks survive a clear");
        s.write(0x18, 0).unwrap();
        assert_eq!(s.read_u32(0x18).unwrap(), 0x7, "byte store too");
    }
}
