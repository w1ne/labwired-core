// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32 APB prescalers as the bus sees them: PCLK1 for the timed CAN bus,
//! and the timer kernel clock (TIMxCLK) every general-purpose and advanced
//! timer counts.

use super::SystemBus;

/// Which APB bus a peripheral's clock gate sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Apb {
    One,
    Two,
}

impl SystemBus {
    /// The divider RCC_CFGR.PPREx puts between HCLK and the APB bus `apb`, or
    /// `None` when the clock controller is not one of the STM32 families
    /// below (or there is none). The family is told apart by where its APB1
    /// enable register sits ([`crate::Peripheral::clock_gate_reg_offset`]):
    ///
    /// - 0x1C, F1 (CFGR 0x04, PPRE1 10:8, PPRE2 13:11, RM0008 §7.3.2), or the
    ///   F0 when it also has CR2 at 0x34 (one PPRE, 10:8, RM0360 §6.4.2);
    /// - 0x40, F4 (CFGR 0x08, PPRE1 12:10, PPRE2 15:13, RM0090 §7.3.3);
    /// - 0x58, L4/L5/G4 (CFGR 0x08, PPRE1 10:8, PPRE2 13:11, RM0351
    ///   §6.4.3, RM0440 §7.4.3);
    /// - 0x3C, G0 (CFGR 0x08, one PPRE, 14:12, RM0444 §5.4.3).
    ///
    /// PPRE 0xx divides by 1, 100 by 2, 101 by 4, 110 by 8, 111 by 16.
    pub(crate) fn stm32_apb_div(&self, apb: Apb) -> Option<u64> {
        let rcc = self.rcc_idx.and_then(|idx| self.peripherals.get(idx))?;
        let f0 = rcc.dev.clock_gate_reg_offset("cr2") == Some(0x34);
        let (cfgr, shift) = match (rcc.dev.clock_gate_reg_offset("apb1enr")?, apb) {
            (0x1C, _) if f0 => (0x04, 8),
            (0x1C, Apb::One) => (0x04, 8),
            (0x1C, Apb::Two) => (0x04, 11),
            (0x40, Apb::One) => (0x08, 10),
            (0x40, Apb::Two) => (0x08, 13),
            (0x58, Apb::One) => (0x08, 8),
            (0x58, Apb::Two) => (0x08, 11),
            (0x3C, _) => (0x08, 12),
            _ => return None,
        };
        let ppre = (rcc.dev.read_u32(cfgr).unwrap_or(0) >> shift) & 0x7;
        Some(if ppre < 4 { 1 } else { 1u64 << (ppre - 3) })
    }

    /// After a store to the clock controller, re-derive every STM32 timer's
    /// kernel clock. TIMxCLK is PCLKx when the APB prescaler is 1 and 2 x
    /// PCLKx otherwise (RM0090 §7.2, RM0008 §7.2), so in HCLK cycles a timer
    /// clock lasts PPRE/2 cycles at APB /4 and slower and one cycle
    /// otherwise. The F4 DCKCFGR.TIMPRE x4 option is not modelled.
    ///
    /// The APB bus is read off the timer's address: every family
    /// [`Self::stm32_apb_div`] knows puts APB1 at 0x4000_0000..0x4000_FFFF
    /// and APB2 at 0x4001_0000..0x4001_FFFF (the G0's single APB spans both,
    /// and both read the one PPRE). A timer outside them is left alone.
    ///
    /// A timer is synced to now before its divider moves, then re-armed, the
    /// same as after a PSC store.
    pub(crate) fn sync_timer_kernel_clocks(&mut self, idx: usize) {
        if self.rcc_idx != Some(idx) {
            return;
        }
        let divs = [Apb::One, Apb::Two].map(|apb| {
            self.stm32_apb_div(apb)
                .map(|d| if d <= 2 { 1 } else { (d / 2) as u32 })
        });
        if divs == [None, None] {
            return;
        }
        let now = self.current_cycle;
        for t in 0..self.peripherals.len() {
            let apb = match self.peripherals[t].base >> 16 {
                0x4000 => Apb::One,
                0x4001 => Apb::Two,
                _ => continue,
            };
            let Some(div) = divs[apb as usize] else {
                continue;
            };
            let p = &mut self.peripherals[t];
            if !p.dev.has_timer_kernel_clock() {
                continue;
            }
            if p.dev.uses_scheduler() {
                p.dev.sync_to(now);
            }
            if p.dev.set_timer_kernel_clock_div(div) {
                p.ticks_remaining = 0;
                self.finish_motor_control_write(t);
                self.collect_scheduled_events(t);
            }
        }
    }
}
