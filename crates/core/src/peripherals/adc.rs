// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::str::FromStr;

/// ADC register layout selector. STM32F1/F2/F4 share the legacy ADC
/// (SR/CR1/CR2/DR @ 0x4C). STM32L4/F7/H7/G0 share the modern ADC with
/// ISR/IER/CR/CFGR/CFGR2/SMPR1/2/SQR1-4/DR @ 0x40 and a different
/// bring-up sequence (DEEPPWD, ADVREGEN, ADCAL, ADEN).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdcRegisterLayout {
    #[default]
    Stm32F1,
    /// STM32L4 family. Hardware-validated against NUCLEO-L476RG.
    Stm32L4,
}

impl FromStr for AdcRegisterLayout {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32l4" | "l4" | "stm32f7" | "f7" | "stm32h7" | "h7" | "stm32g0" | "g0" => {
                Ok(Self::Stm32L4)
            }
            _ => Err(format!(
                "unsupported ADC register layout '{}'; supported: stm32f1, stm32l4",
                value
            )),
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Adc {
    pub layout: AdcRegisterLayout,
    // Legacy F1 registers
    pub sr: u32,  // 0x00
    pub cr1: u32, // 0x04
    pub cr2: u32, // 0x08
    pub dr: u32,  // 0x4C (F1) / 0x40 (L4)

    // L4-only registers
    pub isr: u32,    // 0x00 (L4)
    pub ier: u32,    // 0x04 (L4)
    pub cr: u32,     // 0x08 (L4)
    pub cfgr: u32,   // 0x0C (L4)
    pub cfgr2: u32,  // 0x10 (L4)
    pub smpr1: u32,  // 0x14 (L4)
    pub smpr2: u32,  // 0x18 (L4)
    pub sqr1: u32,   // 0x30 (L4)
    pub sqr2: u32,   // 0x34 (L4)
    pub sqr3: u32,   // 0x38 (L4)
    pub sqr4: u32,   // 0x3C (L4)
    pub common_ccr: u32, // 0x308 (L4 common, ADC_CCR)

    // Internal state (legacy)
    converting: bool,
    cycles_remaining: u32,
    conversion_time: u32,
}

impl Adc {
    pub fn new() -> Self {
        Self::new_with_layout(AdcRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: AdcRegisterLayout) -> Self {
        // Per RM0351 §16.7, the L4 ADC powers up with DEEPPWD set in CR
        // (bit 29 = 0x20000000) and JQDIS set in CFGR (bit 31 =
        // 0x80000000). Verified against real NUCLEO-L476RG silicon via
        // SWD register dump. F1 reset is all-zeros.
        let (cr_reset, cfgr_reset) = match layout {
            AdcRegisterLayout::Stm32F1 => (0, 0),
            AdcRegisterLayout::Stm32L4 => (0x2000_0000, 0x8000_0000),
        };
        Self {
            layout,
            sr: 0,
            cr1: 0,
            cr2: 0,
            dr: 0,
            isr: 0,
            ier: 0,
            cr: cr_reset,
            cfgr: cfgr_reset,
            cfgr2: 0,
            smpr1: 0,
            smpr2: 0,
            sqr1: 0,
            sqr2: 0,
            sqr3: 0,
            sqr4: 0,
            common_ccr: 0,
            converting: false,
            cycles_remaining: 0,
            conversion_time: 14,
        }
    }
}

impl Default for Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Adc {
    fn start_conversion(&mut self) {
        self.converting = true;
        self.cycles_remaining = self.conversion_time;
        // Clear EOC bit on start
        self.sr &= !0x2;
    }
}

impl Adc {
    fn read_reg_l4(&self, reg: u64) -> u32 {
        match reg {
            0x00 => self.isr,
            0x04 => self.ier,
            0x08 => self.cr,
            0x0C => self.cfgr,
            0x10 => self.cfgr2,
            0x14 => self.smpr1,
            0x18 => self.smpr2,
            0x30 => self.sqr1,
            0x34 => self.sqr2,
            0x38 => self.sqr3,
            0x3C => self.sqr4,
            0x40 => self.dr,
            // ADC common block — only CCR modelled.
            0x308 => self.common_ccr,
            _ => 0,
        }
    }

    fn write_reg_l4(&mut self, reg: u64, value: u32) {
        match reg {
            0x00 => {
                // ISR is rc_w1 (write 1 to clear) for most flags. Allow
                // any write to clear matched bits; do not let firmware
                // SET ISR bits explicitly.
                self.isr &= !value;
            }
            0x04 => self.ier = value,
            0x08 => {
                // CR has write semantics that depend on the previous
                // state — ADCAL self-clears when calibration finishes,
                // ADEN by ADRDY, etc. For register-fidelity we just
                // latch what firmware writes. ADCAL stays set until
                // either calibration completes (which requires a real
                // ADC clock — not modelled) or firmware reads it back
                // and observes it cleared. Real silicon with no ADC
                // clock leaves ADCAL set indefinitely — we match that.
                self.cr = value;
            }
            0x0C => self.cfgr = value,
            0x10 => self.cfgr2 = value,
            0x14 => self.smpr1 = value,
            0x18 => self.smpr2 = value,
            0x30 => self.sqr1 = value,
            0x34 => self.sqr2 = value,
            0x38 => self.sqr3 = value,
            0x3C => self.sqr4 = value,
            0x40 => {} // DR is read-only
            0x308 => self.common_ccr = value,
            _ => {}
        }
    }
}

impl Peripheral for Adc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let val = match self.layout {
            AdcRegisterLayout::Stm32F1 => match offset {
                0x00..=0x03 => self.sr,
                0x04..=0x07 => self.cr1,
                0x08..=0x0B => self.cr2,
                0x4C..=0x4F => self.dr,
                _ => 0,
            },
            AdcRegisterLayout::Stm32L4 => self.read_reg_l4(offset & !3),
        };
        let shift = (offset % 4) * 8;
        Ok(((val >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset % 4) * 8;
        let mask: u32 = 0xFF << shift;
        let val_shifted = (value as u32) << shift;

        match self.layout {
            AdcRegisterLayout::Stm32F1 => match offset {
                0x00..=0x03 => {
                    self.sr = (self.sr & !mask) | val_shifted;
                }
                0x04..=0x07 => {
                    self.cr1 = (self.cr1 & !mask) | val_shifted;
                }
                0x08..=0x0B => {
                    let old_cr2 = self.cr2;
                    self.cr2 = (self.cr2 & !mask) | val_shifted;
                    let adon = (self.cr2 & 1) != 0;
                    let swstart = (self.cr2 & (1 << 30)) != 0;
                    let old_swstart = (old_cr2 & (1 << 30)) != 0;
                    if adon && swstart && !old_swstart {
                        self.start_conversion();
                        self.cr2 &= !(1 << 30);
                    }
                }
                _ => {}
            },
            AdcRegisterLayout::Stm32L4 => {
                let reg = offset & !3;
                let mut full = self.read_reg_l4(reg);
                full = (full & !mask) | val_shifted;
                self.write_reg_l4(reg, full);
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut irq = false;
        let mut cycles = 0;

        if self.converting {
            cycles = 1; // It costs 1 cycle to process conversion
            if self.cycles_remaining > 0 {
                self.cycles_remaining -= 1;
            } else {
                // Conversion Complete
                self.converting = false;

                // Mock result: 12-bit value (0..4095).
                // Just increment DR for visual feedback or random.
                self.dr = (self.dr + 1) & 0xFFF;

                // Set EOC (Bit 1)
                self.sr |= 1 << 1;

                // Generate Interrupt if EOCIE (Bit 5 in CR1) is set
                if (self.cr1 & (1 << 5)) != 0 {
                    irq = true;
                }

                // Check Continuous Mode (CONT = Bit 1 in CR2)
                if (self.cr2 & (1 << 1)) != 0 && (self.cr2 & 1) != 0 {
                    // Restart conversion
                    self.start_conversion();
                }
            }
        }

        PeripheralTickResult {
            irq,
            cycles,
            ..Default::default()
        }
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
mod tests {
    use super::*;

    #[test]
    fn test_adc_basic_conversion() {
        let mut adc = Adc::new();
        // Enable ADON and SWSTART (Bit 30)
        adc.write(0x08, 1).unwrap(); // ADON
        adc.write(0x0B, 1 << 6).unwrap(); // SWSTART

        // Should be converting
        assert!(adc.converting);
        assert_eq!(adc.cycles_remaining, 14);

        // Tick 14 times
        for _ in 0..14 {
            let res = adc.tick();
            assert!(adc.converting);
            assert!(!res.irq);
        }

        // Final tick
        let _res = adc.tick();
        assert!(!adc.converting);
        assert_eq!(adc.dr, 1);
        assert!((adc.sr & (1 << 1)) != 0); // EOC bit
    }

    #[test]
    fn test_adc_interrupt() {
        let mut adc = Adc::new();
        // Enable EOCIE (Bit 5 in CR1)
        adc.write(0x04, 1 << 5).unwrap();
        // Start conversion
        adc.write(0x08, 1).unwrap(); // ADON
        adc.write(0x0B, 1 << 6).unwrap(); // SWSTART

        // Finish conversion
        for _ in 0..15 {
            let res = adc.tick();
            if !adc.converting {
                assert!(res.irq);
                return;
            }
        }
        panic!("ADC failed to complete conversion");
    }
}
