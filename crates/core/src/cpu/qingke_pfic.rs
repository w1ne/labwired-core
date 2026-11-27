// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WCH QingKe Programmable Fast Interrupt Controller (PFIC) at 0xE000E000,
//! as on the CH32V002/V003/V004/V005/V006/V007 (QingKe V2) and the
//! CH32V20x/V30x (QingKe V4). The two share this register map; they differ
//! in the registers the hardware prologue saves ([`HPE_REGS`],
//! [`HPE_REGS_V4`]).
//!
//! Sources: WCH noneos-sdk `Core/ch32v00x/core_riscv.h` (`PFIC_Type`,
//! `NVIC_EnableIRQ`/`NVIC_SetPriority`), `Startup/startup_ch32v00x.S` (mtvec
//! `_start | 3`, INTSYSCR = 3) and the CH32V003 Reference Manual, chapter
//! "PFIC". QingKe V4: WCH "QingKe V4 Microprocessor Manual" chapter 3 (PFIC
//! and HPE: 16 caller-saved registers in one cycle) and ch32fun
//! `ch32v20xhw.h` (`PFIC_Type`, same offsets). The window is decoded by the
//! core, as the PLIC and CLINT are.
//!
//! Modelled:
//! - `ISR[8]` (+0x000, enabled status), `IPR[8]` (+0x020, pending status);
//! - `ITHRESDR` (+0x040): a non-zero threshold masks every interrupt whose
//!   priority value is greater than or equal to it;
//! - `GISR` (+0x04C): `NESTSTA` [7:0] active depth, `GACTSTA` bit 8,
//!   `GPENDSTA` bit 9;
//! - `IENR[8]` (+0x100) / `IRER[8]` (+0x180): write 1 to enable / disable;
//! - `IPSR[8]` (+0x200) / `IPRR[8]` (+0x280): write 1 to pend / un-pend;
//! - `IACTR[8]` (+0x300): active status;
//! - `IPRIOR[256]` (+0x400): one priority byte per interrupt, byte-writable.
//!   The lowest value wins; equal values go to the lowest number.
//!
//! Stored only: `CFGR` (+0x048), `VTFIDR` (+0x050), `VTFADDR[4]` (+0x060),
//! `SCTLR` (+0xD10). NOT modelled: the CFGR `KEYCODE` system reset, the
//! vector-table-free (VTF) fast path, and preemption of a running handler
//! (INESTEN): the core clears `mstatus.MIE` on entry, so a pending interrupt
//! waits for `mret`.

/// Base address of the PFIC window.
pub const PFIC_BASE: u32 = 0xE000_E000;
/// Size of the PFIC window. The STK at 0xE000F000 is a separate bus peripheral.
pub const PFIC_SIZE: u32 = 0x1000;

const ISR: u32 = 0x000;
const IPR: u32 = 0x020;
const ITHRESDR: u32 = 0x040;
const CFGR: u32 = 0x048;
const GISR: u32 = 0x04C;
const VTFIDR: u32 = 0x050;
const VTFADDR: u32 = 0x060;
const IENR: u32 = 0x100;
const IRER: u32 = 0x180;
const IPSR: u32 = 0x200;
const IPRR: u32 = 0x280;
const IACTR: u32 = 0x300;
const IPRIOR: u32 = 0x400;
const SCTLR: u32 = 0xD10;

/// Registers the QingKe V2 hardware prologue saves: the RV32E caller-saved
/// set x1, x5..x7, x10..x15.
pub const HPE_REGS: [usize; 10] = [1, 5, 6, 7, 10, 11, 12, 13, 14, 15];

/// Registers the QingKe V4 hardware prologue saves: the RV32I caller-saved
/// set x1, x5..x7, x10..x17, x28..x31.
pub const HPE_REGS_V4: [usize; 16] = [1, 5, 6, 7, 10, 11, 12, 13, 14, 15, 16, 17, 28, 29, 30, 31];

/// INTSYSCR (CSR 0x804) `HWSTKEN`: hardware prologue/epilogue enable.
pub const INTSYSCR_HWSTKEN: u32 = 1 << 0;
/// INTSYSCR `INESTEN`: interrupt nesting enable (stored, see module docs).
pub const INTSYSCR_INESTEN: u32 = 1 << 1;

/// One trap the core entered while the PFIC was attached: the interrupt it
/// marked active (`None` for an exception) and the registers the hardware
/// prologue saved (`None` when HWSTKEN was clear), in the order of the
/// core's HPE register set; a V2 core fills the first ten slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PficFrame {
    pub irq: Option<u32>,
    pub saved: Option<[u32; 16]>,
}

/// QingKe V2 PFIC state (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QingKePfic {
    enabled: [u32; 8],
    pending: [u32; 8],
    active: [u32; 8],
    priority: [u8; 256],
    ithresdr: u32,
    cfgr: u32,
    vtfidr: u32,
    vtfaddr: [u32; 4],
    sctlr: u32,
    /// Traps entered and not yet returned from, innermost last.
    pub frames: Vec<PficFrame>,
}

impl Default for QingKePfic {
    fn default() -> Self {
        Self::new()
    }
}

impl QingKePfic {
    pub fn new() -> Self {
        Self {
            enabled: [0; 8],
            pending: [0; 8],
            active: [0; 8],
            priority: [0; 256],
            ithresdr: 0,
            cfgr: 0,
            vtfidr: 0,
            vtfaddr: [0; 4],
            sctlr: 0,
            frames: Vec::new(),
        }
    }

    /// The PFIC a chip descriptor's core complex carries: every CH32V00x
    /// part (QingKe V2A/V2C), keyed by chip name as the PLIC is, and every
    /// chip that declares a QingKe V4 core (`core: qingke-v4b`, the
    /// ch32-rs/ch32-data name).
    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> bool {
        chip.name.to_ascii_lowercase().starts_with("ch32v00")
            || crate::cpu::RiscVCoreProfile::for_chip(chip)
                == crate::cpu::RiscVCoreProfile::QingkeV4
    }

    /// Offset of `addr` in the PFIC window, if it falls inside.
    #[inline]
    pub fn offset(addr: u32) -> Option<u32> {
        let off = addr.wrapping_sub(PFIC_BASE);
        (off < PFIC_SIZE).then_some(off)
    }

    /// A peripheral asserts interrupt `irq`: latch it pending.
    pub fn raise(&mut self, irq: u32) {
        if irq < 256 {
            self.pending[(irq / 32) as usize] |= 1 << (irq % 32);
        }
    }

    pub fn is_pending(&self, irq: u32) -> bool {
        irq < 256 && self.pending[(irq / 32) as usize] & (1 << (irq % 32)) != 0
    }

    pub fn is_active(&self, irq: u32) -> bool {
        irq < 256 && self.active[(irq / 32) as usize] & (1 << (irq % 32)) != 0
    }

    /// The interrupt the core would take now: pending, enabled, not already
    /// active and below the threshold, lowest priority value first, then
    /// lowest number.
    pub fn next_irq(&self) -> Option<u32> {
        let mut best: Option<(u8, u32)> = None;
        for word in 0..8 {
            let mut ready = self.pending[word] & self.enabled[word] & !self.active[word];
            while ready != 0 {
                let bit = ready.trailing_zeros();
                ready &= ready - 1;
                let irq = word as u32 * 32 + bit;
                let prio = self.priority[irq as usize];
                if self.ithresdr != 0 && u32::from(prio) >= self.ithresdr {
                    continue;
                }
                if best.is_none_or(|(p, _)| prio < p) {
                    best = Some((prio, irq));
                }
            }
        }
        best.map(|(_, irq)| irq)
    }

    /// True when some interrupt is ready to be taken.
    #[inline]
    pub fn has_ready(&self) -> bool {
        (0..8).any(|w| self.pending[w] & self.enabled[w] & !self.active[w] != 0)
    }

    /// Exception entry for `irq`: clear its pending bit, mark it active.
    pub fn take(&mut self, irq: u32) {
        let (w, b) = ((irq / 32) as usize, 1u32 << (irq % 32));
        self.pending[w] &= !b;
        self.active[w] |= b;
    }

    /// `mret`: pop the innermost frame and clear its active bit.
    pub fn complete(&mut self) -> Option<PficFrame> {
        let frame = self.frames.pop()?;
        if let Some(irq) = frame.irq {
            self.active[(irq / 32) as usize] &= !(1 << (irq % 32));
        }
        Some(frame)
    }

    fn gisr(&self) -> u32 {
        let depth = self.frames.iter().filter(|f| f.irq.is_some()).count() as u32;
        let act = self.active.iter().any(|&w| w != 0);
        let pend = (0..8).any(|w| self.pending[w] & self.enabled[w] != 0);
        (depth & 0xFF) | (u32::from(act) << 8) | (u32::from(pend) << 9)
    }

    fn read_word(&self, off: u32) -> u32 {
        let idx = ((off & 0x1F) / 4) as usize;
        match off & !3 {
            o if (ISR..ISR + 0x20).contains(&o) => self.enabled[idx],
            o if (IPR..IPR + 0x20).contains(&o) => self.pending[idx],
            ITHRESDR => self.ithresdr,
            CFGR => self.cfgr,
            GISR => self.gisr(),
            VTFIDR => self.vtfidr,
            o if (VTFADDR..VTFADDR + 0x10).contains(&o) => {
                self.vtfaddr[((o - VTFADDR) / 4) as usize]
            }
            o if (IACTR..IACTR + 0x20).contains(&o) => self.active[idx],
            o if (IPRIOR..IPRIOR + 0x100).contains(&o) => {
                let i = (o - IPRIOR) as usize;
                u32::from_le_bytes([
                    self.priority[i],
                    self.priority[i + 1],
                    self.priority[i + 2],
                    self.priority[i + 3],
                ])
            }
            SCTLR => self.sctlr,
            // IENR/IRER/IPSR/IPRR are write-only and read as zero.
            _ => 0,
        }
    }

    /// Load of `size` bytes at `off`.
    pub fn read(&self, off: u32, size: u32) -> u32 {
        let v = self.read_word(off) >> ((off & 3) * 8);
        if size >= 4 {
            v
        } else {
            v & ((1u32 << (size * 8)) - 1)
        }
    }

    /// Store of `size` bytes at `off`. Priority bytes are written one by one;
    /// every other register takes the value shifted into its byte lanes.
    pub fn write(&mut self, off: u32, size: u32, value: u32) {
        if (IPRIOR..IPRIOR + 0x100).contains(&off) {
            for i in 0..size.min(4) {
                let at = (off - IPRIOR + i) as usize;
                if at < 256 {
                    self.priority[at] = (value >> (i * 8)) as u8;
                }
            }
            return;
        }
        let v = if size >= 4 {
            value
        } else {
            (value & ((1u32 << (size * 8)) - 1)) << ((off & 3) * 8)
        };
        let word = off & !3;
        let idx = ((word & 0x1F) / 4) as usize;
        match word {
            ITHRESDR => self.ithresdr = v & 0xFF,
            CFGR => self.cfgr = v,
            VTFIDR => self.vtfidr = v,
            o if (VTFADDR..VTFADDR + 0x10).contains(&o) => {
                self.vtfaddr[((o - VTFADDR) / 4) as usize] = v
            }
            o if (IENR..IENR + 0x20).contains(&o) => self.enabled[idx] |= v,
            o if (IRER..IRER + 0x20).contains(&o) => self.enabled[idx] &= !v,
            o if (IPSR..IPSR + 0x20).contains(&o) => self.pending[idx] |= v,
            o if (IPRR..IPRR + 0x20).contains(&o) => self.pending[idx] &= !v,
            SCTLR => self.sctlr = v,
            // ISR/IPR/GISR/IACTR are read-only here.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_enable_pend_take_complete() {
        let mut p = QingKePfic::new();
        p.raise(12);
        assert_eq!(p.next_irq(), None, "disabled interrupt is not taken");
        // NVIC_EnableIRQ(SysTicK_IRQn): IENR[0] = 1 << 12.
        p.write(IENR, 4, 1 << 12);
        assert_eq!(p.read(ISR, 4), 1 << 12);
        assert_eq!(p.read(IPR, 4), 1 << 12);
        assert_eq!(p.next_irq(), Some(12));
        p.take(12);
        p.frames.push(PficFrame {
            irq: Some(12),
            saved: None,
        });
        assert_eq!(p.read(IACTR, 4), 1 << 12);
        assert_eq!(p.read(GISR, 4), 1 | 1 << 8);
        p.raise(12);
        assert_eq!(p.next_irq(), None, "an active interrupt is not re-entered");
        assert_eq!(p.complete().and_then(|f| f.irq), Some(12));
        assert_eq!(p.read(IACTR, 4), 0);
        assert_eq!(p.next_irq(), Some(12), "the re-raise waits for mret");
        // NVIC_DisableIRQ: IRER[0] = 1 << 12.
        p.write(IRER, 4, 1 << 12);
        assert_eq!(p.next_irq(), None);
        p.write(IPRR, 4, 1 << 12);
        assert!(!p.is_pending(12));
    }

    #[test]
    fn priority_bytes_threshold_and_tie_break() {
        let mut p = QingKePfic::new();
        p.write(IENR + 4, 4, (1 << 0) | (1 << 6)); // IRQ 32 (USART1), 38 (TIM2)
        p.write(IPSR + 4, 4, (1 << 0) | (1 << 6));
        assert_eq!(p.next_irq(), Some(32), "equal priority: lowest number");
        // NVIC_SetPriority(TIM2_IRQn, 0x40) is a byte store.
        p.write(IPRIOR + 32, 1, 0x80);
        p.write(IPRIOR + 38, 1, 0x40);
        assert_eq!(p.read(IPRIOR + 36, 4), 0x0040_0000);
        assert_eq!(p.next_irq(), Some(38));
        p.write(ITHRESDR, 4, 0x40);
        assert_eq!(p.next_irq(), None, "both at or above the threshold");
        p.write(ITHRESDR, 4, 0x41);
        assert_eq!(p.next_irq(), Some(38));
    }

    #[test]
    fn window_decodes_pfic_only() {
        assert_eq!(QingKePfic::offset(0xE000_E100), Some(0x100));
        assert_eq!(
            QingKePfic::offset(0xE000_F000),
            None,
            "STK is a bus peripheral"
        );
        assert_eq!(QingKePfic::offset(0xE000_DFFF), None);
    }
}
