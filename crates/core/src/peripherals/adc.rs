// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// The family-specific CONTROL registers live in the `AdcRegs` enum: an F1 ADC
// carries only CR1/CR2, an L4 ADC carries only ISR/IER/CR/CFGR/…/CCR — neither
// holds the other's. The data register `dr`, the legacy status `sr` (both poked
// directly by the WASM value-injection bridge), the conversion engine and the
// per-channel injected inputs are architecture-independent and stay shared.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdcRegisterLayout {
    #[default]
    Stm32F1,
    Stm32L4,
}

impl FromStr for AdcRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
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

/// STM32F1 ADC control registers (status `sr` + data `dr` are shared on `Adc`).
#[derive(Debug, Default, serde::Serialize)]
pub struct F1AdcRegs {
    cr1: u32, // 0x04
    cr2: u32, // 0x08
}

/// STM32L4 ADC register file (data `dr` is shared on `Adc`).
#[derive(Debug, Default, serde::Serialize)]
pub struct L4AdcRegs {
    isr: u32,        // 0x00
    ier: u32,        // 0x04
    cr: u32,         // 0x08
    cfgr: u32,       // 0x0C
    cfgr2: u32,      // 0x10
    smpr1: u32,      // 0x14
    smpr2: u32,      // 0x18
    sqr1: u32,       // 0x30
    sqr2: u32,       // 0x34
    sqr3: u32,       // 0x38
    sqr4: u32,       // 0x3C
    common_ccr: u32, // 0x308
}

/// Family-isolated ADC control registers.
#[derive(Debug, serde::Serialize)]
enum AdcRegs {
    Stm32F1(F1AdcRegs),
    Stm32L4(L4AdcRegs),
}

#[derive(Debug, serde::Serialize)]
pub struct Adc {
    regs: AdcRegs,
    /// Legacy status register (F1 SR). Shared because the WASM bridge pokes it
    /// directly to inject an EOC; also the conversion engine's EOC flag.
    pub sr: u32,
    /// Conversion data register — shared result path for both families.
    pub dr: u32,

    // Shared conversion engine.
    converting: bool,
    cycles_remaining: u32,
    conversion_time: u32,
    /// Per-channel injected values (12-bit counts). 0xFFFF = "no injection".
    channel_inputs: [u16; 18],
}

impl Adc {
    pub fn new() -> Self {
        Self::new_with_layout(AdcRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: AdcRegisterLayout) -> Self {
        // Per RM0351 §16.7 the L4 ADC powers up with DEEPPWD set in CR
        // (bit 29) and JQDIS set in CFGR (bit 31) — verified on NUCLEO-L476RG.
        // F1 reset is all-zeros.
        let regs = match layout {
            AdcRegisterLayout::Stm32F1 => AdcRegs::Stm32F1(F1AdcRegs::default()),
            AdcRegisterLayout::Stm32L4 => AdcRegs::Stm32L4(L4AdcRegs {
                cr: 0x2000_0000,
                cfgr: 0x8000_0000,
                ..Default::default()
            }),
        };
        Self {
            regs,
            sr: 0,
            dr: 0,
            converting: false,
            cycles_remaining: 0,
            conversion_time: 14,
            channel_inputs: [0xFFFF; 18],
        }
    }

    /// Inject a millivolt reading for a specific ADC channel. The next
    /// conversion on this channel returns the equivalent 12-bit count.
    pub fn set_channel_input(&mut self, channel: u8, millivolts: u16) {
        if (channel as usize) < self.channel_inputs.len() {
            let count = ((millivolts as u32 * 4095) / 3300).min(4095) as u16;
            self.channel_inputs[channel as usize] = count;
        }
    }

    fn start_conversion(&mut self) {
        self.converting = true;
        self.cycles_remaining = self.conversion_time;
        self.sr &= !0x2; // clear EOC on start
    }

    /// (cr1, cr2) for the F1 control registers; (0, 0) on L4 (no conversion
    /// engine runs there, so these are only consulted on the F1 path).
    fn f1_ctrl(&self) -> (u32, u32) {
        match &self.regs {
            AdcRegs::Stm32F1(r) => (r.cr1, r.cr2),
            AdcRegs::Stm32L4(_) => (0, 0),
        }
    }

    fn read_reg_l4(r: &L4AdcRegs, dr: u32, reg: u64) -> u32 {
        match reg {
            0x00 => r.isr,
            0x04 => r.ier,
            0x08 => r.cr,
            0x0C => r.cfgr,
            0x10 => r.cfgr2,
            0x14 => r.smpr1,
            0x18 => r.smpr2,
            0x30 => r.sqr1,
            0x34 => r.sqr2,
            0x38 => r.sqr3,
            0x3C => r.sqr4,
            0x40 => dr,
            0x308 => r.common_ccr,
            _ => 0,
        }
    }

    fn write_reg_l4(r: &mut L4AdcRegs, reg: u64, value: u32) {
        match reg {
            // ISR is rc_w1 — a write clears matched flags; firmware can't SET it.
            0x00 => r.isr &= !value,
            0x04 => r.ier = value,
            0x08 => r.cr = value, // latch verbatim (ADCAL/ADEN self-clear not modelled)
            0x0C => r.cfgr = value,
            0x10 => r.cfgr2 = value,
            0x14 => r.smpr1 = value,
            0x18 => r.smpr2 = value,
            0x30 => r.sqr1 = value,
            0x34 => r.sqr2 = value,
            0x38 => r.sqr3 = value,
            0x3C => r.sqr4 = value,
            0x40 => {} // DR read-only
            0x308 => r.common_ccr = value,
            _ => {}
        }
    }
}

impl Default for Adc {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Adc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let val = match &self.regs {
            AdcRegs::Stm32F1(r) => match offset {
                0x00..=0x03 => self.sr,
                0x04..=0x07 => r.cr1,
                0x08..=0x0B => r.cr2,
                0x4C..=0x4F => self.dr,
                _ => 0,
            },
            AdcRegs::Stm32L4(r) => Self::read_reg_l4(r, self.dr, offset & !3),
        };
        let shift = (offset % 4) * 8;
        Ok(((val >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset % 4) * 8;
        let mask: u32 = 0xFF << shift;
        let val_shifted = (value as u32) << shift;

        match self.regs {
            AdcRegs::Stm32F1(_) => match offset {
                0x00..=0x03 => self.sr = (self.sr & !mask) | val_shifted,
                0x04..=0x07 => {
                    if let AdcRegs::Stm32F1(r) = &mut self.regs {
                        r.cr1 = (r.cr1 & !mask) | val_shifted;
                    }
                }
                0x08..=0x0B => {
                    // Update CR2; decide whether to kick off a conversion, then
                    // release the `regs` borrow before calling start_conversion
                    // (which mutates the shared engine fields).
                    let mut trigger = false;
                    if let AdcRegs::Stm32F1(r) = &mut self.regs {
                        let old_cr2 = r.cr2;
                        r.cr2 = (r.cr2 & !mask) | val_shifted;
                        let adon = (r.cr2 & 1) != 0;
                        let swstart = (r.cr2 & (1 << 30)) != 0;
                        let old_swstart = (old_cr2 & (1 << 30)) != 0;
                        if adon && swstart && !old_swstart {
                            r.cr2 &= !(1 << 30);
                            trigger = true;
                        }
                    }
                    if trigger {
                        self.start_conversion();
                    }
                }
                _ => {}
            },
            AdcRegs::Stm32L4(_) => {
                let reg = offset & !3;
                let dr = self.dr;
                if let AdcRegs::Stm32L4(r) = &mut self.regs {
                    let full = (Self::read_reg_l4(r, dr, reg) & !mask) | val_shifted;
                    Self::write_reg_l4(r, reg, full);
                }
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let mut irq = false;
        let mut cycles = 0;

        if self.converting {
            cycles = 1;
            if self.cycles_remaining > 0 {
                self.cycles_remaining -= 1;
            } else {
                self.converting = false;
                let (cr1, cr2) = self.f1_ctrl();

                // Injected channel value if available; else increment DR for
                // visual feedback. SQR3 ch fallback uses CR2 low bits (legacy).
                let ch = (cr2 & 0x1F) as usize;
                if ch < self.channel_inputs.len() && self.channel_inputs[ch] != 0xFFFF {
                    self.dr = self.channel_inputs[ch] as u32;
                } else {
                    self.dr = (self.dr + 1) & 0xFFF;
                }

                self.sr |= 1 << 1; // EOC

                if (cr1 & (1 << 5)) != 0 {
                    irq = true; // EOCIE
                }
                // Continuous mode (CONT bit1 + ADON bit0).
                if (cr2 & (1 << 1)) != 0 && (cr2 & 1) != 0 {
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
        adc.write(0x08, 1).unwrap(); // ADON
        adc.write(0x0B, 1 << 6).unwrap(); // SWSTART (bit 30)

        assert!(adc.converting);
        assert_eq!(adc.cycles_remaining, 14);

        for _ in 0..14 {
            let res = adc.tick();
            assert!(adc.converting);
            assert!(!res.irq);
        }

        let _res = adc.tick();
        assert!(!adc.converting);
        assert_eq!(adc.dr, 1);
        assert!((adc.sr & (1 << 1)) != 0); // EOC
    }

    #[test]
    fn test_adc_interrupt() {
        let mut adc = Adc::new();
        adc.write(0x04, 1 << 5).unwrap(); // EOCIE
        adc.write(0x08, 1).unwrap(); // ADON
        adc.write(0x0B, 1 << 6).unwrap(); // SWSTART

        for _ in 0..15 {
            let res = adc.tick();
            if !adc.converting {
                assert!(res.irq);
                return;
            }
        }
        panic!("ADC failed to complete conversion");
    }

    #[test]
    fn test_adc_l4_reset_values() {
        let adc = Adc::new_with_layout(AdcRegisterLayout::Stm32L4);
        // CR (0x08) DEEPPWD=bit29, CFGR (0x0C) JQDIS=bit31 — silicon-verified.
        let cr = (adc.read(0x08).unwrap() as u32)
            | (adc.read(0x09).unwrap() as u32) << 8
            | (adc.read(0x0A).unwrap() as u32) << 16
            | (adc.read(0x0B).unwrap() as u32) << 24;
        assert_eq!(cr, 0x2000_0000);
        let cfgr = (adc.read(0x0C).unwrap() as u32) | (adc.read(0x0F).unwrap() as u32) << 24;
        assert_eq!(cfgr & 0x8000_0000, 0x8000_0000);
    }
}
