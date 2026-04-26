// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// STM32 timer peripheral covering basic, general-purpose, and advanced-
/// control variants:
///
/// - **Basic** (TIM6/TIM7): CR1/DIER/SR/EGR/CNT/PSC/ARR only.
/// - **General-purpose** (TIM2/3/4/5): adds CR2/SMCR/CCMR1/2/CCER + 4
///   capture/compare channels (CCR1..CCR4). TIM2/TIM5 are 32-bit (set
///   `width: 32` in YAML); TIM3/TIM4 are 16-bit.
/// - **Advanced-control** (TIM1/TIM8): general-purpose plus RCR
///   (repetition counter), BDTR (break + dead-time + MOE master output
///   enable), CCMR3, CCR5/CCR6, OR1/OR2. Set `advanced: true` in YAML.
///   The MOE bit in BDTR gates all output channels — without it asserted,
///   PWM outputs stay in their idle state regardless of CCER configuration.
///
/// Counter / ARR width (16 or 32) is selectable via `width: 32` in the
/// chip yaml's `config` block.
#[derive(Debug, Default, serde::Serialize)]
pub struct Timer {
    cr1: u32,
    cr2: u32,
    smcr: u32,
    dier: u32,
    sr: u32,
    egr: u32,
    ccmr1: u32,
    ccmr2: u32,
    ccer: u32,
    cnt: u32,
    psc: u32,
    arr: u32,
    rcr: u32,
    ccr1: u32,
    ccr2: u32,
    ccr3: u32,
    ccr4: u32,
    bdtr: u32,
    dcr: u32,
    dmar: u32,
    or1: u32,
    ccmr3: u32,
    ccr5: u32,
    ccr6: u32,
    or2: u32,

    /// Counter / ARR width (16 or 32). Defaults to 16 for back-compat
    /// with existing F1-class chip configs.
    width: u8,

    /// Whether this instance has the advanced-control register set
    /// (TIM1/TIM8). Gates the RCR/BDTR/CCMR3/CCR5-6/OR1-2 fields.
    advanced: bool,

    // Internal state
    psc_cnt: u32,
}

impl Timer {
    pub fn new() -> Self {
        Self::new_with_width(16)
    }

    pub fn new_with_width(width: u8) -> Self {
        Self::new_with_layout(width, false)
    }

    pub fn new_with_layout(width: u8, advanced: bool) -> Self {
        let arr_reset = if width >= 32 { 0xFFFF_FFFF } else { 0xFFFF };
        Self {
            cr1: 0,
            cr2: 0,
            smcr: 0,
            dier: 0,
            sr: 0,
            egr: 0,
            ccmr1: 0,
            ccmr2: 0,
            ccer: 0,
            cnt: 0,
            psc: 0,
            arr: arr_reset,
            rcr: 0,
            ccr1: 0,
            ccr2: 0,
            ccr3: 0,
            ccr4: 0,
            // BDTR resets to 0 — MOE deasserted, PWM outputs gated until
            // firmware explicitly sets BDTR.MOE bit 15.
            bdtr: 0,
            dcr: 0,
            dmar: 0,
            or1: 0,
            ccmr3: 0,
            ccr5: 0,
            ccr6: 0,
            or2: 0,
            width,
            advanced,
            psc_cnt: 0,
        }
    }

    fn cnt_mask(&self) -> u32 {
        if self.width >= 32 { 0xFFFF_FFFF } else { 0xFFFF }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.smcr,
            0x0C => self.dier,
            0x10 => self.sr,
            0x14 => self.egr,
            0x18 => self.ccmr1,
            0x1C => self.ccmr2,
            0x20 => self.ccer,
            0x24 => self.cnt,
            0x28 => self.psc,
            0x2C => self.arr,
            0x30 if self.advanced => self.rcr,
            0x34 => self.ccr1,
            0x38 => self.ccr2,
            0x3C => self.ccr3,
            0x40 => self.ccr4,
            0x44 if self.advanced => self.bdtr,
            0x48 => self.dcr,
            0x4C => self.dmar,
            0x50 if self.advanced => self.or1,
            0x54 if self.advanced => self.ccmr3,
            0x58 if self.advanced => self.ccr5,
            0x5C if self.advanced => self.ccr6,
            0x60 if self.advanced => self.or2,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr1 = value & 0x3FF,
            0x04 => self.cr2 = value & 0x00FF_FFFB,
            0x08 => self.smcr = value & 0xFFFF_FFF7,
            0x0C => self.dier = value & 0x7FFF,
            // TIMx_SR is rc_w0 for status flags: writing 0 clears, writing 1 keeps current.
            0x10 => self.sr &= value & 0x1FFFF,
            // TIMx_EGR: only UG (bit 0) drives state — but advanced timers
            // also have CC1G-CC4G, COMG, TG, BG. We accept all and treat
            // CC*G as also setting the corresponding CC*IF flags.
            0x14 => {
                self.egr = value & 0xFF;
                if (self.egr & 0x01) != 0 {
                    self.cnt = 0;
                    self.psc_cnt = 0;
                    self.sr |= 1; // UIF
                }
                if self.advanced {
                    if (self.egr & 0x02) != 0 { self.sr |= 1 << 1; } // CC1IF
                    if (self.egr & 0x04) != 0 { self.sr |= 1 << 2; }
                    if (self.egr & 0x08) != 0 { self.sr |= 1 << 3; }
                    if (self.egr & 0x10) != 0 { self.sr |= 1 << 4; }
                    if (self.egr & 0x80) != 0 { self.sr |= 1 << 7; } // BIF (break)
                }
            }
            0x18 => self.ccmr1 = value,
            0x1C => self.ccmr2 = value,
            0x20 => {
                // CCER mask: general-purpose timers expose CCxE (bit 0) +
                // CCxP (bit 1) per channel — 4 channels = 0x3333.
                // Advanced timers add CCxNE (bit 2) + CCxNP (bit 3) for
                // the complementary output — 4 channels = 0xFFFF.
                let mask = if self.advanced { 0xFFFF } else { 0x3333 };
                self.ccer = value & mask;
            }
            0x24 => self.cnt = value & self.cnt_mask(),
            0x28 => self.psc = value & 0xFFFF,
            0x2C => self.arr = value & self.cnt_mask(),
            0x30 if self.advanced => self.rcr = value & 0xFFFF,
            0x34 => self.ccr1 = value & self.cnt_mask(),
            0x38 => self.ccr2 = value & self.cnt_mask(),
            0x3C => self.ccr3 = value & self.cnt_mask(),
            0x40 => self.ccr4 = value & self.cnt_mask(),
            // BDTR: full register, including MOE (bit 15) which gates PWM
            // outputs. Real silicon has lock-protection for some bits via
            // LOCK[1:0]; we accept all writes for survival-mode firmware.
            0x44 if self.advanced => self.bdtr = value & 0x03FF_FFFF,
            0x48 => self.dcr = value & 0x1F1F,
            0x4C => self.dmar = value,
            0x50 if self.advanced => self.or1 = value,
            0x54 if self.advanced => self.ccmr3 = value,
            0x58 if self.advanced => self.ccr5 = value,
            0x5C if self.advanced => self.ccr6 = value,
            0x60 if self.advanced => self.or2 = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for Timer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);

        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        // Keep IRQ level high while UIF is latched and UIE is enabled.
        if (self.sr & 1) != 0 && (self.dier & 1) != 0 {
            return crate::PeripheralTickResult {
                irq: true,
                cycles: 1,
                ..Default::default()
            };
        }

        // Counter Enable (bit 0)
        if (self.cr1 & 0x1) == 0 {
            return crate::PeripheralTickResult {
                irq: false,
                cycles: 0,
                ..Default::default()
            };
        }

        self.psc_cnt = self.psc_cnt.wrapping_add(1);
        if self.psc_cnt > self.psc {
            self.psc_cnt = 0;
            self.cnt = self.cnt.wrapping_add(1);

            if self.cnt > self.arr {
                self.cnt = 0;
                self.sr |= 1; // Set UIF (Update Interrupt Flag)

                // Return true if Update Interrupt Enable (UIE) is set
                return crate::PeripheralTickResult {
                    irq: (self.dier & 1) != 0,
                    cycles: 1,
                    dma_signals: None,
                    ..Default::default()
                };
            }
        }

        crate::PeripheralTickResult {
            irq: false,
            cycles: 0,
            dma_signals: None,
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Timer;
    use crate::Peripheral;

    #[test]
    fn test_egr_ug_sets_uif_and_cnt_reset() {
        let mut tim = Timer::new();
        tim.write(0x24, 0x34).unwrap(); // CNT low byte
        tim.write(0x25, 0x12).unwrap(); // CNT high byte => 0x1234

        tim.write(0x14, 0x01).unwrap(); // EGR.UG

        let cnt_lo = tim.read(0x24).unwrap();
        let cnt_hi = tim.read(0x25).unwrap();
        let sr = tim.read(0x10).unwrap();
        assert_eq!((cnt_hi as u16) << 8 | cnt_lo as u16, 0);
        assert_eq!(sr & 0x1, 0x1);
    }

    #[test]
    fn test_sr_write_zero_clears_uif_and_drops_irq() {
        let mut tim = Timer::new();

        // Enable UIE and set UIF via UG.
        tim.write(0x0C, 0x01).unwrap();
        tim.write(0x14, 0x01).unwrap();
        assert!(tim.tick().irq);

        // Clear UIF by writing 0 to SR bit 0.
        tim.write(0x10, 0x00).unwrap();
        assert_eq!(tim.read(0x10).unwrap() & 0x1, 0);
        assert!(!tim.tick().irq);
    }

    #[test]
    fn test_advanced_bdtr_round_trips_moe() {
        let mut tim = Timer::new_with_layout(16, true);
        // Enable MOE (bit 15) + dead-time generator value 0x40.
        tim.write(0x44, 0x40).unwrap();
        tim.write(0x45, 0x80).unwrap();
        let bdtr_lo = tim.read(0x44).unwrap();
        let bdtr_hi = tim.read(0x45).unwrap();
        assert_eq!(bdtr_lo, 0x40);
        assert_eq!(bdtr_hi, 0x80);
    }

    #[test]
    fn test_advanced_rcr_writes_persisted() {
        let mut tim = Timer::new_with_layout(16, true);
        tim.write(0x30, 0x05).unwrap();
        assert_eq!(tim.read(0x30).unwrap(), 0x05);
    }

    #[test]
    fn test_basic_timer_ignores_advanced_regs() {
        let mut tim = Timer::new_with_layout(16, false);
        tim.write(0x44, 0x80).unwrap(); // BDTR — should no-op
        assert_eq!(tim.read(0x44).unwrap(), 0x00);
        tim.write(0x30, 0x05).unwrap(); // RCR — should no-op
        assert_eq!(tim.read(0x30).unwrap(), 0x00);
    }
}
