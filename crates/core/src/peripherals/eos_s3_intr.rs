// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! QuickLogic EOS S3 interrupt controller (`INTR_CTRL`, 0x4000_4800), the
//! block Renode maps as `IRQControllers.EOSS3_IntrCtrl` in
//! `platforms/cpus/eos-s3.repl`.
//!
//! Sources: qorc-sdk `HAL/inc/eoss3_dev.h` (`INTR_CTRL_TypeDef`, offsets
//! 0x00..0xA4; `SW_INTR_1` / `SW_INTR_2` bit 0; `IRQn_Type`:
//! `SwInt2_IRQn = 0`, `SwInt1_IRQn = 1`) and Renode `EOSS3_IntrCtrl.cs`
//! (SoftwareIrq2 -> NVIC 0, SoftwareIrq1 -> NVIC 1; `SW_INTR_n` reads and
//! writes the active flag; the M4 enable gates the line).
//!
//! Modelled:
//! - `SOFTWARE_INTR_1` (0x40) / `SOFTWARE_INTR_2` (0x50) bit 0: written 1
//!   sets the request, written 0 clears it.
//! - `SOFTWARE_INTR_n_EN_M4` (0x48 / 0x58) bit 0 gates it. The line
//!   `SW_INTR_n & EN_M4` is a LEVEL on NVIC 1 (SW1) / NVIC 0 (SW2): it is
//!   pended when it rises, and the pend is dropped when firmware clears
//!   the request or the enable.
//! - The configuration words are stored at their documented widths:
//!   `GPIO_INTR_TYPE` / `_POL` / `_EN_AP` / `_EN_M4` / `_EN_FFE0` /
//!   `_EN_FFE1` (8 bits), `OTHER_INTR_EN_AP` / `_EN_M4` (24 bits),
//!   `SOFTWARE_INTR_n_EN_AP` (bit 0), `FFE_INTR_EN_*` and
//!   `FFE1_FB_INTR_EN_*` (8 bits), and the `FB_INTR_TYPE` / `_POL` /
//!   `_EN_*` and `M4_MEM_AON_INTR_EN` words (32 bits; no width published).
//!
//! NOT modelled: every interrupt SOURCE except the two software requests.
//! `GPIO_INTR`, `GPIO_INTR_RAW`, `OTHER_INTR`, `FFE_INTR`, `FFE1_FB_INTR`,
//! `FB_INTR`, `FB_INTR_RAW` and `M4_MEM_AON_INTR` read 0 and ignore
//! writes: no GPIO edge or level is detected, and no peripheral (PL011,
//! timer, watchdog, ...) is routed through `OTHER_INTR`, so NVIC 3..29
//! never assert from this block. The AP and FFE outputs go nowhere. The
//! re-entry when a handler returns with the request still set is not
//! modelled: the line is pended again only when it rises again.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// Renode `Size => 0x400`.
const WORDS: usize = 0x400 / 4;

const SW1: usize = 0x40 / 4;
const SW1_EN_M4: usize = 0x48 / 4;
const SW2: usize = 0x50 / 4;
const SW2_EN_M4: usize = 0x58 / 4;

/// NVIC lines (qorc-sdk `SwInt1_IRQn`, `SwInt2_IRQn`).
pub const SW1_IRQ: u32 = 1;
pub const SW2_IRQ: u32 = 0;

/// Event token for "raise the lines that just rose".
const RAISE_TOKEN: u32 = 0;

/// Writable mask of each stored word; 0 = reads 0, ignores writes.
fn word_mask(index: usize) -> u32 {
    match index * 4 {
        // GPIO_INTR_TYPE, _POL, _EN_AP, _EN_M4, _EN_FFE0, _EN_FFE1.
        0x08..=0x1C => 0xFF,
        // OTHER_INTR_EN_AP, _EN_M4.
        0x34 | 0x38 => 0x00FF_FFFF,
        // SOFTWARE_INTR_n and their AP / M4 enables.
        0x40 | 0x44 | 0x48 | 0x50 | 0x54 | 0x58 => 0x1,
        // FFE_INTR_EN_AP / _M4, FFE1_FB_INTR_EN_AP / _M4.
        0x64 | 0x68 | 0x74 | 0x78 => 0xFF,
        // FB_INTR_TYPE, _POL, _EN_AP, _EN_M4; M4_MEM_AON_INTR_EN.
        0x88..=0x94 | 0xA4 => 0xFFFF_FFFF,
        _ => 0,
    }
}

/// Status words that exist but have no modelled source (read 0).
fn is_unsourced_status(index: usize) -> bool {
    matches!(
        index * 4,
        0x00 | 0x04 | 0x30 | 0x60 | 0x70 | 0x80 | 0x84 | 0xA0
    )
}

/// EOS S3 `INTR_CTRL` (see the module docs).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EosS3IntrCtrl {
    regs: Vec<u32>,
    /// NVIC lines that rose since the last delivery.
    raise: Vec<u32>,
    /// A raise event is queued for the scheduler.
    #[serde(skip)]
    pending_event: bool,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Default for EosS3IntrCtrl {
    fn default() -> Self {
        Self::new()
    }
}

impl EosS3IntrCtrl {
    pub fn new() -> Self {
        Self {
            regs: vec![0; WORDS],
            raise: Vec::new(),
            pending_event: false,
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn sw1_line(&self) -> bool {
        self.regs[SW1] & self.regs[SW1_EN_M4] & 1 != 0
    }

    fn sw2_line(&self) -> bool {
        self.regs[SW2] & self.regs[SW2_EN_M4] & 1 != 0
    }

    fn read_word(&self, index: usize) -> u32 {
        self.regs.get(index).copied().unwrap_or(0)
    }

    fn write_word(&mut self, index: usize, value: u32) {
        let mask = word_mask(index);
        if mask == 0 {
            if !is_unsourced_status(index) {
                crate::census_reg!(
                    "eos_s3_intr_ctrl:EosS3IntrCtrl",
                    (index * 4) as u64,
                    "write"
                );
            }
            return;
        }
        let (was1, was2) = (self.sw1_line(), self.sw2_line());
        self.regs[index] = value & mask;
        if !was1 && self.sw1_line() {
            self.raise.push(SW1_IRQ);
        }
        if !was2 && self.sw2_line() {
            self.raise.push(SW2_IRQ);
        }
    }

    fn take_raise(&mut self) -> Option<Vec<u32>> {
        // A line that rose and fell before delivery is not pended.
        let (l1, l2) = (self.sw1_line(), self.sw2_line());
        let mut lines = std::mem::take(&mut self.raise);
        lines.retain(|&l| (l == SW1_IRQ && l1) || (l == SW2_IRQ && l2));
        lines.dedup();
        (!lines.is_empty()).then_some(lines)
    }
}

impl Peripheral for EosS3IntrCtrl {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word((offset / 4) as usize) >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let index = (offset / 4) as usize;
        let shift = (offset & 3) * 8;
        let merged = (self.read_word(index) & !(0xFF << shift)) | (u32::from(value) << shift);
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

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && !self.raise.is_empty()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    /// Both software lines are levels: clearing the request (or its M4
    /// enable) inside the handler drops a pend that is still waiting.
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        report(SW1_IRQ, self.sw1_line());
        report(SW2_IRQ, self.sw2_line());
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        PeripheralTickResult {
            explicit_irqs: self.take_raise(),
            ..Default::default()
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if self.scheduler_mode() && !self.raise.is_empty() && !self.pending_event {
            self.pending_event = true;
            vec![(0, RAISE_TOKEN)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        self.pending_event = false;
        crate::sched::EventResult {
            explicit_irqs: self.take_raise().unwrap_or_default(),
            ..Default::default()
        }
    }

    fn on_system_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new();
        self.clock = clock;
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qorc_layout_widths() {
        let mut c = EosS3IntrCtrl::new();
        c.write_u32(0x14, 0xFFFF_FFFF).unwrap();
        assert_eq!(c.read_u32(0x14).unwrap(), 0xFF, "GPIO_INTR_EN_M4");
        c.write_u32(0x38, 0xFFFF_FFFF).unwrap();
        assert_eq!(c.read_u32(0x38).unwrap(), 0x00FF_FFFF, "OTHER_INTR_EN_M4");
        c.write_u32(0x44, 0xFFFF_FFFF).unwrap();
        assert_eq!(c.read_u32(0x44).unwrap(), 1, "SW_INTR_1_EN_AP");
        for status in [0x00, 0x04, 0x30, 0x60, 0x70, 0x80, 0x84, 0xA0] {
            c.write_u32(status, 0xFFFF_FFFF).unwrap();
            assert_eq!(c.read_u32(status).unwrap(), 0, "status 0x{status:02X}");
        }
    }

    #[test]
    fn software_request_pends_only_when_enabled() {
        let mut c = EosS3IntrCtrl::new();
        c.write_u32(0x40, 1).unwrap();
        assert_eq!(c.read_u32(0x40).unwrap(), 1);
        assert_eq!(c.tick().explicit_irqs, None, "EN_M4 clear");
        c.write_u32(0x48, 1).unwrap();
        assert_eq!(c.tick().explicit_irqs, Some(vec![SW1_IRQ]));
        assert_eq!(c.tick().explicit_irqs, None, "pended once per rise");
        c.write_u32(0x58, 1).unwrap();
        c.write_u32(0x50, 1).unwrap();
        assert_eq!(c.tick().explicit_irqs, Some(vec![SW2_IRQ]));
    }

    #[test]
    fn clear_drops_the_level_and_a_cancelled_rise() {
        let mut c = EosS3IntrCtrl::new();
        c.write_u32(0x48, 1).unwrap();
        c.write_u32(0x40, 1).unwrap();
        let mut levels = Vec::new();
        c.irq_line_levels(&mut |l, v| levels.push((l, v)));
        assert_eq!(levels, vec![(SW1_IRQ, true), (SW2_IRQ, false)]);
        c.write_u32(0x40, 0).unwrap();
        assert_eq!(
            c.tick().explicit_irqs,
            None,
            "rose and fell before delivery"
        );
        levels.clear();
        c.irq_line_levels(&mut |l, v| levels.push((l, v)));
        assert_eq!(levels, vec![(SW1_IRQ, false), (SW2_IRQ, false)]);
    }

    #[test]
    fn scheduler_mode_queues_one_raise_event() {
        if !crate::cycle_clock::event_scheduler_enabled() {
            return;
        }
        let mut c = EosS3IntrCtrl::new();
        c.attach_cycle_clock(CycleClock::default());
        c.write_u32(0x58, 1).unwrap();
        c.write_u32(0x50, 1).unwrap();
        assert_eq!(c.tick().explicit_irqs, None, "walk is skipped");
        assert_eq!(c.take_scheduled_events(), vec![(0, RAISE_TOKEN)]);
        assert!(c.take_scheduled_events().is_empty());
        assert_eq!(c.take_raise(), Some(vec![SW2_IRQ]));
    }

    #[test]
    fn system_reset_clears_requests() {
        let mut c = EosS3IntrCtrl::new();
        c.write_u32(0x48, 1).unwrap();
        c.write_u32(0x40, 1).unwrap();
        c.on_system_reset();
        assert_eq!(c.read_u32(0x40).unwrap(), 0);
        assert_eq!(c.read_u32(0x48).unwrap(), 0);
        assert_eq!(c.tick().explicit_irqs, None);
    }
}
