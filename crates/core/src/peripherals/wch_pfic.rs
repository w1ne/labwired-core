// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WCH QingKe V2 Programmable Fast Interrupt Controller (PFIC) — the CH32V003
//! interrupt controller at `0xE000_E000`.
//!
//! ## Register file (WCH `core_riscv.h`, `PFIC_Type`, CH32V00x)
//!
//! | off   | reg      | here                                                |
//! |-------|----------|-----------------------------------------------------|
//! | 0x000 | ISR[8]   | read: the enable bits (`IENR` state)                |
//! | 0x020 | IPR[8]   | read: the pending bits                              |
//! | 0x040 | ITHRESDR | stored, not applied                                 |
//! | 0x048 | CFGR     | reads 0; the reset key is not modelled              |
//! | 0x04C | GISR     | reads 0; not modelled                               |
//! | 0x050 | VTFIDR   | stored; fast-interrupt vectoring is not modelled    |
//! | 0x060 | VTFADDR  | stored; see above                                   |
//! | 0x100 | IENR[8]  | write 1 enables                                     |
//! | 0x180 | IRER[8]  | write 1 disables                                    |
//! | 0x200 | IPSR[8]  | write 1 pends                                       |
//! | 0x280 | IPRR[8]  | write 1 un-pends                                    |
//! | 0x300 | IACTR[8] | read: active bits                                   |
//! | 0x400 | IPRIOR   | one priority byte per interrupt number              |
//! | 0xD10 | SCTLR    | stored (WFI→WFE / SEVONPEND bits are not applied)   |
//!
//! From `IENR` up, the PFIC lays its words out exactly as the ARMv7-M NVIC
//! lays out ISER/ICER/ISPR/ICPR/IABR/IPR, each 0x100 higher. So that half is
//! served by [`Nvic`] over a shared [`NvicState`], and the bus's existing
//! pend / level-reconcile paths (`pend_nvic`, `reconcile_nvic_level`) drive
//! it unchanged. One difference: a PFIC bit position IS the interrupt number
//! (`IENR[0]` bit 12 is `SysTicK_IRQn = 12`, as `NVIC_EnableIRQ` writes it).
//! There is no `-16` offset. A peripheral entry's `irq:` is that number.
//!
//! The CPU side (taking the interrupt, the `mtvec` mode-3 vector fetch, IACTR
//! on entry/`mret`) lives in `cpu/riscv.rs`, which holds the same state.

use crate::peripherals::nvic::{Nvic, NvicState};
use crate::{Peripheral, SimResult};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// The PFIC window: `0xE000_E000..0xE000_F000` (SCTLR is at 0xD10).
pub const WCH_PFIC_SIZE: u64 = 0x1000;

const ISR: u64 = 0x000;
const IPR: u64 = 0x020;
const ITHRESDR: u64 = 0x040;
const VTFIDR: u64 = 0x050;
const VTFADDR: u64 = 0x060;
/// `IENR`: the first word the NVIC bank serves.
const NVIC_BANK: u64 = 0x100;
const SCTLR: u64 = 0xD10;

#[derive(Debug)]
pub struct WchPfic {
    nvic: Nvic,
    ithresdr: u32,
    vtfidr: u32,
    vtfaddr: [u32; 4],
    sctlr: u32,
}

impl Default for WchPfic {
    fn default() -> Self {
        Self::new()
    }
}

impl WchPfic {
    pub fn new() -> Self {
        Self {
            nvic: Nvic::new(Arc::new(NvicState::default())),
            ithresdr: 0,
            vtfidr: 0,
            vtfaddr: [0; 4],
            sctlr: 0,
        }
    }

    /// The enable/pending/active/priority state the bus pends into and the
    /// QingKe core takes interrupts from.
    pub fn state(&self) -> Arc<NvicState> {
        self.nvic.state.clone()
    }

    fn read_word(&self, off: u64) -> u32 {
        let s = &self.nvic.state;
        let word = |bank: &[std::sync::atomic::AtomicU32; 8], base: u64| {
            bank[((off - base) / 4) as usize].load(Ordering::SeqCst)
        };
        match off {
            ISR..=0x01C => word(&s.iser, ISR),
            IPR..=0x03C => word(&s.ispr, IPR),
            ITHRESDR => self.ithresdr,
            VTFIDR => self.vtfidr,
            VTFADDR..=0x06C => self.vtfaddr[((off - VTFADDR) / 4) as usize],
            SCTLR => self.sctlr,
            _ => 0,
        }
    }

    fn write_word(&mut self, off: u64, value: u32) {
        match off {
            ITHRESDR => self.ithresdr = value,
            VTFIDR => self.vtfidr = value,
            VTFADDR..=0x06C => self.vtfaddr[((off - VTFADDR) / 4) as usize] = value,
            SCTLR => self.sctlr = value,
            _ => {}
        }
    }
}

impl Peripheral for WchPfic {
    // A register bank over shared state: the pend scan runs in the bus and the
    // core, never in a tick.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        if (NVIC_BANK..SCTLR).contains(&offset) {
            return self.nvic.read(offset - NVIC_BANK);
        }
        let w = self.read_word(offset & !3);
        Ok((w >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if (NVIC_BANK..SCTLR).contains(&offset) {
            return self.nvic.write(offset - NVIC_BANK, value);
        }
        let aligned = offset & !3;
        let sh = (offset & 3) * 8;
        let cur = self.read_word(aligned);
        self.write_word(aligned, (cur & !(0xFFu32 << sh)) | ((value as u32) << sh));
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        self.nvic.snapshot()
    }

    fn wch_pfic_state(&self) -> Option<Arc<NvicState>> {
        Some(self.state())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write32(p: &mut WchPfic, off: u64, v: u32) {
        for i in 0..4 {
            p.write(off + i, (v >> (i * 8)) as u8).unwrap();
        }
    }

    fn read32(p: &WchPfic, off: u64) -> u32 {
        (0..4).fold(0, |acc, i| {
            acc | (p.read(off + i).unwrap() as u32) << (i * 8)
        })
    }

    /// `NVIC_EnableIRQ(SysTicK_IRQn)` writes `IENR[0] = 1 << 12`; ISR reads it back.
    #[test]
    fn ienr_enables_by_interrupt_number_and_isr_reads_it() {
        let mut p = WchPfic::new();
        write32(&mut p, 0x100, 1 << 12);
        assert_eq!(read32(&p, ISR), 1 << 12);
        assert_eq!(p.state().iser[0].load(Ordering::SeqCst), 1 << 12);
        write32(&mut p, 0x180, 1 << 12);
        assert_eq!(read32(&p, ISR), 0);
    }

    #[test]
    fn ipsr_pends_and_iprr_clears_through_ipr() {
        let mut p = WchPfic::new();
        write32(&mut p, 0x204, 1 << 6); // interrupt 38 (TIM2)
        assert_eq!(read32(&p, IPR + 4), 1 << 6);
        write32(&mut p, 0x284, 1 << 6);
        assert_eq!(read32(&p, IPR + 4), 0);
    }

    /// `NVIC_SetPriority(SysTicK_IRQn, 0xF0)` is a byte store to IPRIOR[12].
    #[test]
    fn iprior_is_one_byte_per_interrupt_number() {
        let mut p = WchPfic::new();
        p.write(0x400 + 12, 0xF0).unwrap();
        assert_eq!(p.state().ipr_priority(12), 0xF0);
        assert_eq!(p.state().ipr_priority(13), 0);
    }

    #[test]
    fn sctlr_and_ithresdr_are_stored() {
        let mut p = WchPfic::new();
        write32(&mut p, SCTLR, 1 << 3);
        write32(&mut p, ITHRESDR, 0x80);
        assert_eq!(read32(&p, SCTLR), 1 << 3);
        assert_eq!(read32(&p, ITHRESDR), 0x80);
    }
}
