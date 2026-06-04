// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// EXTI is one struct PER FAMILY behind the `Exti` enum. The F1 variant is a
// single 20-line bank; the L4 variant adds bank 2 (lines 32..39). The bank-2
// registers therefore exist ONLY on the L4 variant — an F1 EXTI cannot carry
// (or be tricked into addressing) bank-2 state. Bank-1 IRQ routing, shared by
// both families, lives in one stateless helper.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtiRegisterLayout {
    /// STM32F1 / F4-class: single bank, 20 lines, registers at 0x00..0x14.
    #[default]
    Stm32F1,
    /// STM32L4: two banks (40 lines total). Bank1 at 0x00..0x14, bank2 at
    /// 0x20..0x34. Bank-2 covers lines 32..39 (LPTIM/COMP/I2C/USART wakeup).
    Stm32L4,
}

impl FromStr for ExtiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32l4" | "l4" => Ok(Self::Stm32L4),
            _ => Err(format!(
                "unsupported EXTI register layout '{}'; supported: stm32f1, stm32l4",
                value
            )),
        }
    }
}

/// One EXTI register bank (IMR/EMR/RTSR/FTSR/SWIER/PR).
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
struct ExtiBank {
    imr: u32,
    emr: u32,
    rtsr: u32,
    ftsr: u32,
    swier: u32,
    pr: u32,
}

impl ExtiBank {
    /// IMR/EMR/RTSR/FTSR read at 0x00/0x04/0x08/0x0C; SWIER 0x10; PR 0x14.
    fn read(&self, off: u64) -> u32 {
        match off {
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0C => self.ftsr,
            0x10 => self.swier,
            0x14 => self.pr,
            _ => 0,
        }
    }
    /// `mask` is the implemented-line mask for this bank.
    fn write(&mut self, off: u64, value: u32, mask: u32) {
        match off {
            0x00 => self.imr = value & mask,
            0x04 => self.emr = value & mask,
            0x08 => self.rtsr = value & mask,
            0x0C => self.ftsr = value & mask,
            0x10 => {
                // SWIER: a 0->1 edge sets the matching PR bit.
                let diff = (self.swier ^ value) & value & mask;
                self.swier = value & mask;
                self.pr |= diff;
            }
            0x14 => self.pr &= !(value & mask), // rc_w1
            _ => {}
        }
    }
}

/// Bank-1 IRQ routing — identical on every family (lines 0..4 → IRQ 6..10,
/// 9..5 → 23, 15..10 → 40). Shared behaviour, not shared state.
fn route_bank1_irqs(active1: u32, irqs: &mut Vec<u32>) {
    for i in 0..5 {
        if (active1 & (1 << i)) != 0 {
            irqs.push(6 + i);
        }
    }
    if (active1 & 0x0000_03E0) != 0 {
        irqs.push(23); // EXTI9_5
    }
    if (active1 & 0x0000_FC00) != 0 {
        irqs.push(40); // EXTI15_10
    }
}

// ── STM32F1 / F4: single 20-line bank ────────────────────────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct F1Exti {
    bank1: ExtiBank,
}

impl F1Exti {
    const MASK: u32 = 0x000F_FFFF; // 20 lines
}

// ── STM32L4: two banks (bank 2 = lines 32..39) ───────────────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct L4Exti {
    bank1: ExtiBank,
    bank2: ExtiBank,
}

impl L4Exti {
    const MASK1: u32 = 0xFFFF_FFFF; // full word for bank 1
    const MASK2: u32 = 0x0000_00FF; // lines 32..39
}

/// External Interrupt/Event Controller — one variant per chip family.
#[derive(Debug, serde::Serialize)]
pub enum Exti {
    Stm32F1(F1Exti),
    Stm32L4(L4Exti),
}

impl Default for Exti {
    fn default() -> Self {
        Self::new()
    }
}

impl Exti {
    pub fn new() -> Self {
        Self::new_with_layout(ExtiRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: ExtiRegisterLayout) -> Self {
        match layout {
            ExtiRegisterLayout::Stm32F1 => Self::Stm32F1(F1Exti::default()),
            ExtiRegisterLayout::Stm32L4 => Self::Stm32L4(L4Exti::default()),
        }
    }

    /// Inject an external trigger on `line` (sets the corresponding PR bit).
    /// Bank-2 lines (32..39) exist only on the L4 variant.
    pub fn trigger_line(&mut self, line: u8) {
        match self {
            Self::Stm32F1(e) => {
                if line < 32 {
                    e.bank1.pr |= 1u32 << line;
                }
            }
            Self::Stm32L4(e) => match line {
                0..=31 => e.bank1.pr |= 1u32 << line,
                32..=39 => e.bank2.pr |= 1u32 << (line - 32),
                _ => {}
            },
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match self {
            Self::Stm32F1(e) => match offset {
                0x00..=0x14 => e.bank1.read(offset),
                _ => 0,
            },
            Self::Stm32L4(e) => match offset {
                0x00..=0x14 => e.bank1.read(offset),
                0x20..=0x34 => e.bank2.read(offset - 0x20),
                _ => 0,
            },
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match self {
            Self::Stm32F1(e) => {
                if (0x00..=0x14).contains(&offset) {
                    e.bank1.write(offset, value, F1Exti::MASK);
                }
            }
            Self::Stm32L4(e) => match offset {
                0x00..=0x14 => e.bank1.write(offset, value, L4Exti::MASK1),
                0x20..=0x34 => e.bank2.write(offset - 0x20, value, L4Exti::MASK2),
                _ => {}
            },
        }
    }
}

impl Peripheral for Exti {
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

    fn tick(&mut self) -> PeripheralTickResult {
        let mut irqs = Vec::new();
        match self {
            Self::Stm32F1(e) => {
                let active1 = e.bank1.pr & e.bank1.imr;
                if active1 != 0 {
                    route_bank1_irqs(active1, &mut irqs);
                }
            }
            Self::Stm32L4(e) => {
                let active1 = e.bank1.pr & e.bank1.imr;
                let active2 = e.bank2.pr & e.bank2.imr;
                if active1 != 0 {
                    route_bank1_irqs(active1, &mut irqs);
                }
                if active2 != 0 {
                    // Bank-2 wakeup lines → their peripheral's NVIC IRQ
                    // (RM0351 §13.3). Lines without an entry are tracked at
                    // the register level but don't synthesize an IRQ yet.
                    for &(line, irq) in &[
                        (35u32, 70u32), // LPUART1 wakeup
                        (36, 31),       // I2C1 wakeup
                        (37, 33),       // I2C2 wakeup
                        (38, 72),       // I2C3 wakeup
                        (39, 37),       // USART1 wakeup
                    ] {
                        if (active2 >> (line - 32)) & 1 != 0 {
                            irqs.push(irq);
                        }
                    }
                }
            }
        }

        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
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
    use super::{Exti, ExtiRegisterLayout};
    use crate::Peripheral;

    fn poke(exti: &mut Exti, off: u64, val: u32) {
        for i in 0..4 {
            exti.write(off + i, ((val >> (i * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    #[test]
    fn l4_bank2_line35_routes_to_lpuart1_irq() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32L4);
        // Arm IMR2 line 3 (= EXTI line 35), then trigger via SWIER2.
        poke(&mut e, 0x20, 1 << 3);
        poke(&mut e, 0x30, 1 << 3);
        let r = e.tick();
        let irqs = r.explicit_irqs.expect("expected IRQ list");
        assert!(
            irqs.contains(&70),
            "LPUART1 IRQ 70 should fire, got {irqs:?}"
        );
    }

    #[test]
    fn l4_bank2_line38_routes_to_i2c3_irq() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32L4);
        poke(&mut e, 0x20, 1 << 6);
        poke(&mut e, 0x30, 1 << 6);
        let r = e.tick();
        let irqs = r.explicit_irqs.expect("expected IRQ list");
        assert!(irqs.contains(&72), "I2C3 IRQ 72 should fire, got {irqs:?}");
    }

    #[test]
    fn f1_layout_does_not_synth_bank2_irqs() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        poke(&mut e, 0x20, 1 << 3); // bank 2 doesn't even exist on F1
        poke(&mut e, 0x30, 1 << 3);
        let r = e.tick();
        // No bank-2 -> no IRQ
        assert!(r.explicit_irqs.is_none() || r.explicit_irqs.unwrap().is_empty());
    }

    #[test]
    fn f1_bank1_swier_sets_pr_and_routes() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        poke(&mut e, 0x00, 1 << 0); // IMR line 0
        poke(&mut e, 0x10, 1 << 0); // SWIER line 0 -> PR
        let r = e.tick();
        assert!(r.explicit_irqs.expect("irqs").contains(&6), "EXTI0 -> IRQ6");
    }
}
