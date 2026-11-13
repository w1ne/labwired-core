// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32H7 ADC common block (ADC12_COMMON / ADC3_COMMON), first modelled for
//! the STM32H723ZG (RM0468 §25.7, `stm32h723xx.h` ADC_Common_TypeDef).
//!
//! On the L4/G4 the common registers sit inside the ADC1 page (+0x300) and
//! the `adc` model answers them. On the H7, ADC2 sits 0x100 above ADC1, so
//! each instance window is 256 bytes and the common block is its own window
//! at ADC1 + 0x300. Stock Cube `HAL_ADC_Init` writes CCR (CKMODE/PRESC) on
//! every `analogRead`, so the block must exist for stock firmware.
//!
//! Registers:
//!   CSR  0x00 — status mirror of the master/slave ISRs (read-only)
//!   CCR  0x08 — common control: DUAL[4:0], DELAY[11:8], DAMDF[15:14],
//!               CKMODE[17:16], PRESC[21:18], VREFEN(22), TSEN(23),
//!               VBATEN(24). Reset 0.
//!   CDR  0x0C — dual-mode packed data (read-only)
//!   CDR2 0x10 — dual-mode 32-bit data (read-only)
//!
//! What is and is not simulated:
//! * CCR keeps exactly its defined fields; reserved bits read 0.
//! * Dual/interleaved modes are not simulated: CSR, CDR and CDR2 read 0, the
//!   value an independent-mode converter pair shows when neither instance
//!   has a pending flag. CSR does not mirror the instance ISRs.
//! * CKMODE/PRESC do not change conversion timing.

use crate::SimResult;

/// CCR defined fields: DUAL[4:0], DELAY[11:8], DAMDF[15:14], CKMODE[17:16],
/// PRESC[21:18], VREFEN(22), TSEN(23), VBATEN(24).
const CCR_WRITABLE_MASK: u32 = 0x01FF_CF1F;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct AdcCommon {
    ccr: u32,
}

impl AdcCommon {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 | 0x0C | 0x10 => 0,
            0x08 => self.ccr,
            _ => {
                crate::census_reg!("adc_common:AdcCommon", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x08 => self.ccr = value & CCR_WRITABLE_MASK,
            0x00 | 0x0C | 0x10 => {} // read-only
            _ => {
                crate::census_reg!("adc_common:AdcCommon", offset, "write");
            }
        }
    }
}

impl crate::Peripheral for AdcCommon {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

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

#[cfg(test)]
mod tests {
    use super::AdcCommon;
    use crate::Peripheral;

    #[test]
    fn reset_values_are_zero() {
        let c = AdcCommon::new();
        for off in [0x00, 0x08, 0x0C, 0x10] {
            assert_eq!(c.read_u32(off).unwrap(), 0, "offset {off:#x}");
        }
    }

    /// `HAL_ADC_Init` on the NUCLEO_H723ZG: LL_ADC_SetCommonClock writes
    /// CKMODE = 0 (async) | PRESC = 0b0001 (div 2) through MODIFY_REG.
    #[test]
    fn ccr_keeps_defined_fields_only() {
        let mut c = AdcCommon::new();
        c.write_u32(0x08, 0x0004_0000).unwrap();
        assert_eq!(c.read_u32(0x08).unwrap(), 0x0004_0000, "PRESC div 2");
        c.write_u32(0x08, 0xFFFF_FFFF).unwrap();
        assert_eq!(c.read_u32(0x08).unwrap(), 0x01FF_CF1F, "reserved bits drop");
    }

    #[test]
    fn status_and_data_are_read_only() {
        let mut c = AdcCommon::new();
        for off in [0x00, 0x0C, 0x10] {
            c.write_u32(off, 0xFFFF_FFFF).unwrap();
            assert_eq!(c.read_u32(off).unwrap(), 0, "offset {off:#x}");
        }
    }
}
