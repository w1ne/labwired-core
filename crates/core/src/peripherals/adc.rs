// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;

#[derive(Debug, serde::Serialize)]
pub struct Adc {
    // Registers
    pub sr: u32,  // 0x00 - Status Register
    pub cr1: u32, // 0x04 - Control Register 1
    pub cr2: u32, // 0x08 - Control Register 2
    pub dr: u32,  // 0x4C - Data Register

    // Internal State
    converting: bool,
    cycles_remaining: u32,
    conversion_time: u32, // Cycles per conversion (e.g. 14)
}

impl Adc {
    pub fn new() -> Self {
        Self {
            sr: 0,
            cr1: 0,
            cr2: 0,
            dr: 0,
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

impl Peripheral for Adc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let val = match offset {
            0x00..=0x03 => self.sr,
            0x04..=0x07 => self.cr1,
            0x08..=0x0B => self.cr2,
            0x4C..=0x4F => self.dr,
            _ => 0,
        };

        let shift = (offset % 4) * 8;
        Ok(((val >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset % 4) * 8;
        let mask = 0xFF << shift;
        let val_shifted = (value as u32) << shift;

        match offset {
            0x00..=0x03 => {
                // SR is generally read-only or rc_w0, but here allowed simple WO for EOC clear
                // If writing 0 to EOC (bit 1), clear it.
                // Standard STM32: rc_w0.
                // For simplicity: Update SR, but EOC is set by hardware.
                // Let's implement Write-1-to-Clear for EOC if needed, or direct write.
                // Spec says: EOC is cleared by reading DR or writing 0 to it.
                self.sr = (self.sr & !mask) | val_shifted;
            }
            0x04..=0x07 => {
                self.cr1 = (self.cr1 & !mask) | val_shifted;
            }
            0x08..=0x0B => {
                let old_cr2 = self.cr2;
                self.cr2 = (self.cr2 & !mask) | val_shifted;

                // Check ADON (Bit 0) and SWSTART (Bit 30)
                let adon = (self.cr2 & 1) != 0;
                let swstart = (self.cr2 & (1 << 30)) != 0;
                let old_swstart = (old_cr2 & (1 << 30)) != 0;

                // If ADON is set and SWSTART transitions 0->1, start conversion
                if adon && swstart && !old_swstart {
                    self.start_conversion();
                    // SWSTART bit is usually cleared by hardware after start?
                    // In STM32F1 it is cleared by hardware.
                    self.cr2 &= !(1 << 30);
                }
            }
            0x4C..=0x4F => {
                // DR is read-only usually
            }
            _ => {}
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
            dma_requests: None,
            explicit_irqs: None,
            dma_signals: None,
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
