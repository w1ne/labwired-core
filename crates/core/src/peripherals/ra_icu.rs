// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA interrupt controller unit, event-link half: `IELSRn`.
//!
//! On RA parts no peripheral is wired to a fixed NVIC line. Each peripheral
//! interrupt is an ELC *event number*, and `IELSRn` (one per NVIC line n)
//! selects which event drives line n. Source: FSP `R7FA8M1AH.h`
//! `R_ICU_Type` (`IELSR[96]` at R_ICU_BASE + 0x6300; `IELS` bits 8:0, `IR`
//! bit 16, `DTCE` bit 24) and FSP `bsp_irq.h` `R_BSP_IrqStatusClear` ("When
//! an interrupt is triggered the IR bit is set. If it is not cleared in the
//! ISR then the interrupt will trigger again immediately.").
//!
//! This model covers the 2 KiB window from R_ICU_BASE + 0x6000 (0x4000_C000
//! on the RA8M1, `WUPEN0/1` at +0x1A0/+0x1A4 and `IELSR0-95` at +0x300).
//! Every word in the window is plain storage; `IELSRn` adds:
//! - an event linked to line n (`IELS == event`, `IELS != 0`) sets `IR` and
//!   drives NVIC line n;
//! - `IR` is the level of line n ([`Peripheral::irq_line_levels`]), so the
//!   bus drops a pend raised by the link once firmware clears `IR`;
//! - `IR` is write-0-to-clear: a store of 1 leaves it as it was.
//!
//! NOT modelled: `DTCE` (stored, no DTC activation), the IRQ-pin half of the
//! ICU (`IRQCRn`/NMI at R_ICU_BASE, still a stub), and the re-entry when a
//! handler returns with `IR` still set — the line is only re-pended by the
//! next linked event, not by the level alone, because walking this block
//! per cycle while any `IR` is held would stop idle fast-forward.

use crate::{Peripheral, SimResult};

/// Window size: R_ICU_BASE + 0x6000 .. + 0x6800.
const WINDOW: usize = 0x800;
/// `IELSR0` offset in the window (R_ICU_BASE + 0x6300).
pub const IELSR_OFFSET: u64 = 0x300;
/// RA8M1 `IELSR` count (NVIC lines 0..95).
pub const LINES: u32 = 96;
const IELS_MASK: u32 = 0x1FF;
const IR: u32 = 1 << 16;
const DTCE: u32 = 1 << 24;

/// The RA ICU event-link registers.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaIcu {
    words: Vec<u32>,
}

impl Default for RaIcu {
    fn default() -> Self {
        Self::new()
    }
}

impl RaIcu {
    pub fn new() -> Self {
        Self {
            words: vec![0; WINDOW / 4],
        }
    }

    fn ielsr_index(offset: u64) -> Option<usize> {
        let rel = offset.checked_sub(IELSR_OFFSET)?;
        let n = rel / 4;
        (n < u64::from(LINES)).then_some(n as usize)
    }

    /// `IELSRn` value.
    pub fn ielsr(&self, n: usize) -> u32 {
        self.words[(IELSR_OFFSET / 4) as usize + n]
    }

    fn load(&self, offset: u64) -> u32 {
        self.words.get((offset / 4) as usize).copied().unwrap_or(0)
    }

    fn store_word(&mut self, offset: u64, value: u32) {
        let w = (offset / 4) as usize;
        if w >= self.words.len() {
            return;
        }
        let value = if Self::ielsr_index(offset & !3).is_some() {
            // IR is write-0-to-clear; IELS and DTCE are plain fields.
            let old = self.words[w];
            (value & (IELS_MASK | DTCE)) | (old & value & IR)
        } else {
            value
        };
        self.words[w] = value;
    }

    /// Sub-word store: merge into the word, then apply the word rules.
    fn store(&mut self, offset: u64, width: u32, value: u32) {
        let shift = (offset & 3) * 8;
        let mask = if width == 4 {
            u32::MAX
        } else {
            ((1u32 << (width * 8)) - 1) << shift
        };
        let word = offset & !3;
        let old = self.load(word);
        // Lanes not written keep their bits, including IR (old & IR = IR).
        self.store_word(word, (old & !mask) | ((value << shift) & mask));
    }
}

impl Peripheral for RaIcu {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.load(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok((self.load(offset & !3) >> ((offset & 2) * 8)) as u16)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.load(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.store(offset, 1, u32::from(value));
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        self.store(offset & !1, 2, u32::from(value));
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.store(offset & !3, 4, value);
        Ok(())
    }

    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        for n in 0..LINES as usize {
            let v = self.ielsr(n);
            if v & IELS_MASK != 0 {
                report(n as u32, v & IR != 0);
            }
        }
    }

    fn link_event(&mut self, event: u32, line: &mut dyn FnMut(u32)) -> bool {
        if event != 0 {
            let first = (IELSR_OFFSET / 4) as usize;
            for n in 0..LINES as usize {
                let w = &mut self.words[first + n];
                if *w & IELS_MASK == event {
                    *w |= IR;
                    line(n as u32);
                }
            }
        }
        true
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        #[allow(unused_mut)]
        let mut restored: RaIcu = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("RaIcu snapshot decode: {error}"))
        })?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGT0_INT: u32 = 0x046;

    fn ielsr_off(n: u64) -> u64 {
        IELSR_OFFSET + 4 * n
    }

    fn linked(icu: &mut RaIcu, event: u32) -> Vec<u32> {
        let mut lines = Vec::new();
        assert!(icu.link_event(event, &mut |n| lines.push(n)));
        lines
    }

    fn levels(icu: &RaIcu) -> Vec<(u32, bool)> {
        let mut out = Vec::new();
        icu.irq_line_levels(&mut |n, l| out.push((n, l)));
        out
    }

    #[test]
    fn a_linked_event_sets_ir_and_drives_its_line() {
        let mut icu = RaIcu::new();
        icu.write_u32(ielsr_off(83), AGT0_INT).unwrap();
        assert_eq!(linked(&mut icu, AGT0_INT), vec![83]);
        assert_eq!(icu.read_u32(ielsr_off(83)).unwrap(), IR | AGT0_INT);
        assert_eq!(levels(&icu), vec![(83, true)]);
        // Another event is not linked anywhere.
        assert!(linked(&mut icu, AGT0_INT + 1).is_empty());
    }

    #[test]
    fn ir_is_write_zero_to_clear() {
        let mut icu = RaIcu::new();
        // Firmware cannot set IR itself.
        icu.write_u32(ielsr_off(5), IR | AGT0_INT).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(5)).unwrap(), AGT0_INT);
        linked(&mut icu, AGT0_INT);
        // A store of 1 keeps it, the R_BSP_IrqStatusClear store of 0 clears it.
        icu.write_u32(ielsr_off(5), IR | AGT0_INT).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(5)).unwrap() & IR, IR);
        icu.write_u32(ielsr_off(5), AGT0_INT).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(5)).unwrap(), AGT0_INT);
        assert_eq!(levels(&icu), vec![(5, false)]);
    }

    #[test]
    fn a_byte_store_to_iels_keeps_ir() {
        let mut icu = RaIcu::new();
        icu.write_u32(ielsr_off(1), AGT0_INT).unwrap();
        linked(&mut icu, AGT0_INT);
        icu.write(ielsr_off(1), 0x47).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(1)).unwrap(), IR | 0x47);
        // The byte holding IR, written with 0, clears it.
        icu.write(ielsr_off(1) + 2, 0).unwrap();
        assert_eq!(icu.read_u32(ielsr_off(1)).unwrap(), 0x47);
    }

    #[test]
    fn one_event_can_drive_several_lines_and_unlinked_lines_report_nothing() {
        let mut icu = RaIcu::new();
        icu.write_u32(ielsr_off(2), AGT0_INT).unwrap();
        icu.write_u32(ielsr_off(40), AGT0_INT | DTCE).unwrap();
        assert_eq!(linked(&mut icu, AGT0_INT), vec![2, 40]);
        assert_eq!(levels(&icu), vec![(2, true), (40, true)]);
        // Event 0 means "no event" (IELS reset value): never linked.
        assert!(linked(&mut icu, 0).is_empty());
    }

    #[test]
    fn other_words_are_storage() {
        let mut icu = RaIcu::new();
        icu.write_u32(0x1A0, 0xDEAD_BEEF).unwrap();
        assert_eq!(icu.read_u32(0x1A0).unwrap(), 0xDEAD_BEEF);
        assert_eq!(icu.read_u32(0x7FC).unwrap(), 0);
    }
}
