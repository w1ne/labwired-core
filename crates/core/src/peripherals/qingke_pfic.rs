// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! WCH QingKe V2 Programmable Fast Interrupt Controller (PFIC), the CH32V003
//! interrupt controller at 0xE000_E000.
//!
//! Register map (WCH `core_riscv.h` `PFIC_Type` for CH32V00x and the
//! CH32V003 reference manual's PFIC chapter). The enable/pending/active
//! banks deliberately sit at the Cortex-M NVIC offsets, but bit `n` of word
//! `n / 32` is interrupt number `n` itself: SysTick is 12, SW is 14 and
//! USART1 is 32, the same numbers as the vector table slots.
//!
//! | Offset | Name | Model |
//! | --- | --- | --- |
//! | 0x000 | `ISR[8]` | read-only enable status |
//! | 0x020 | `IPR[8]` | read-only pending status (latched or level-asserted) |
//! | 0x040 | `ITHRESDR` | priority threshold, bits [7:6] |
//! | 0x048 | `CFGR` | reads 0; the key-gated system reset is not modeled |
//! | 0x04C | `GISR` | NESTSTA [7:0], GACTSTA bit 8, GPENDSTA bit 9 |
//! | 0x050 | `VTFIDR` | VTF interrupt numbers, bytes 0..1 |
//! | 0x060 | `VTFADDR[2]` | VTF handler address, bit 0 enables the channel |
//! | 0x100 | `IENR[8]` | write 1 to enable (reads 0) |
//! | 0x180 | `IRER[8]` | write 1 to disable (reads 0) |
//! | 0x200 | `IPSR[8]` | write 1 to pend (reads 0) |
//! | 0x280 | `IPRR[8]` | write 1 to clear a latched pend (reads 0) |
//! | 0x300 | `IACTR[8]` | read-only active status |
//! | 0x400 | `IPRIOR[256]` | one byte per interrupt, bits [7:6] implemented |
//! | 0xD10 | `SCTLR` | stored; SETEVENT (bit 5) sets the event register, WFITOWFE (bit 3) makes `wfi` a WFE; SLEEPONEXIT/SEVONPEND/SYSRESET have no effect |
//!
//! The registers live in [`QingkePficState`], shared with the RISC-V core
//! (installed by `configure_riscv`) the way the Cortex-M NVIC shares
//! `NvicState`. The core arbitrates and takes the interrupt; the peripheral
//! is only the firmware's MMIO view.
//!
//! Two pend sources exist. A pulse from a bus peripheral (`irq:` on its
//! descriptor) or an `IPSR` write latches a pending bit that interrupt entry
//! clears. A level source — the QingKe SysTick's `STIE && CNTIF` and its
//! `SWIE` software trigger — drives [`QingkePficState::level`] directly, so
//! the interrupt stays requested exactly while the source asserts it and a
//! handler that clears `STK_SR` drops it on the same store.
//!
//! Events (WFE). [`QingkePficState::event`] is the core's event register:
//! an SCTLR store with SETEVENT set, or an EXTI line whose EVENR bit is set
//! seeing its selected edge, raises it. With SCTLR.WFITOWFE set the core's
//! `wfi` consumes a raised event and continues, or else stays parked on the
//! `wfi` until an event arrives or an interrupt the PFIC would deliver is
//! requested (ch32fun's `__WFE()` is SETEVENT, then two `wfi`: the first
//! eats its own event, the second sleeps). EXTI lines that no GPIO pad
//! drives (line 9, the PWR auto-wakeup) reach EXTI through
//! [`QingkePficState::exti_line_pulse`], since EXTI has no tick of its own.
//!
//! Not modeled: interrupt nesting (`INTSYSCR.INESTEN`) — a higher preemption
//! priority waits for the running handler's `mret`; the key-gated system
//! reset in `CFGR`/`SCTLR`; SLEEPONEXIT and SEVONPEND; the difference
//! between sleep and deep sleep/standby (SLEEPDEEP, PWR_CTLR.PDDS): every
//! clock keeps running while the core is parked; the NMI.

use crate::{Peripheral, SimResult};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

const WORDS: usize = 8;
/// IPRIOR bits the CH32V003 implements; the low six read as zero.
const PRIORITY_MASK: u8 = 0xC0;
/// The CH32V003 has two VTF (vector table free) channels.
const VTF_CHANNELS: usize = 2;

/// PFIC register state shared between the MMIO peripheral and the core.
#[derive(Debug)]
pub struct QingkePficState {
    pub enabled: [AtomicU32; WORDS],
    /// Latched pends: IPSR writes and bus peripheral pulses. Cleared on
    /// interrupt entry or by an IPRR write.
    pub pending: [AtomicU32; WORDS],
    /// Requests held by level sources while they assert them.
    pub level: [AtomicU32; WORDS],
    pub active: [AtomicU32; WORDS],
    /// IPRIOR, four interrupts per word, little-endian like the MMIO bytes.
    pub priority: [AtomicU32; 64],
    pub threshold: AtomicU32,
    pub vtf_id: AtomicU32,
    pub vtf_addr: [AtomicU32; VTF_CHANNELS],
    pub sctlr: AtomicU32,
    /// The core's event register (see the module docs).
    pub event: AtomicBool,
    /// EXTI configuration published by the CH32V003 EXTI on every store:
    /// RTENR | FTENR, EVENR and INTENR.
    pub exti_edge: AtomicU32,
    pub exti_evenr: AtomicU32,
    pub exti_intenr: AtomicU32,
    /// INTFR bits latched by [`Self::exti_line_pulse`]; EXTI reads them as
    /// part of INTFR and its write-1-clear drops them.
    pub exti_internal_flags: AtomicU32,
}

/// SCTLR bits (QingKe V2 manual, ch32fun `__WFE`).
const SCTLR_WFITOWFE: u32 = 1 << 3;
const SCTLR_SETEVENT: u32 = 1 << 5;
/// EXTI line 9 is the PWR auto-wakeup; `AWU_IRQn` is 21 (ch32v003hw.h).
const EXTI_AWU_LINE: u32 = 9;
const AWU_IRQ: u32 = 21;

impl Default for QingkePficState {
    fn default() -> Self {
        Self {
            enabled: Default::default(),
            pending: Default::default(),
            level: Default::default(),
            active: Default::default(),
            priority: [0; 64].map(|_| AtomicU32::new(0)),
            threshold: AtomicU32::new(0),
            vtf_id: AtomicU32::new(0),
            vtf_addr: Default::default(),
            sctlr: AtomicU32::new(0),
            event: AtomicBool::new(false),
            exti_edge: AtomicU32::new(0),
            exti_evenr: AtomicU32::new(0),
            exti_intenr: AtomicU32::new(0),
            exti_internal_flags: AtomicU32::new(0),
        }
    }
}

fn split(irq: u32) -> Option<(usize, u32)> {
    let word = (irq / 32) as usize;
    (word < WORDS).then_some((word, 1 << (irq % 32)))
}

impl QingkePficState {
    /// Latch a pend, as a peripheral pulse or an IPSR write does.
    pub fn pend(&self, irq: u32) {
        if let Some((word, bit)) = split(irq) {
            self.pending[word].fetch_or(bit, Ordering::SeqCst);
        }
    }

    /// Drive a level source's request line.
    pub fn set_level(&self, irq: u32, asserted: bool) {
        if let Some((word, bit)) = split(irq) {
            if asserted {
                self.level[word].fetch_or(bit, Ordering::SeqCst);
            } else {
                self.level[word].fetch_and(!bit, Ordering::SeqCst);
            }
        }
    }

    /// Raise the core's event register.
    pub fn raise_event(&self) {
        self.event.store(true, Ordering::SeqCst);
    }

    /// Consume the event register; true when it was raised.
    pub fn take_event(&self) -> bool {
        self.event.swap(false, Ordering::SeqCst)
    }

    /// SCTLR.WFITOWFE: the core's `wfi` waits for an event.
    pub fn wfi_is_wfe(&self) -> bool {
        self.sctlr.load(Ordering::Relaxed) & SCTLR_WFITOWFE != 0
    }

    /// An internal EXTI source (not a GPIO pad) pulsed `line`. As for a pad
    /// edge, nothing happens unless RTENR or FTENR selects the line; then
    /// INTFR latches it, EVENR raises the event register and INTENR the
    /// line's interrupt. The pulse rises and falls, so either edge counts.
    pub fn exti_line_pulse(&self, line: u32) {
        let bit = 1u32 << line;
        if self.exti_edge.load(Ordering::SeqCst) & bit == 0 {
            return;
        }
        self.exti_internal_flags.fetch_or(bit, Ordering::SeqCst);
        if self.exti_evenr.load(Ordering::SeqCst) & bit != 0 {
            self.raise_event();
        }
        self.publish_exti_internal_levels(0);
    }

    /// Drive the interrupts of the non-GPIO EXTI lines from `INTFR & INTENR`,
    /// with `gpio_flags` the EXTI's own INTFR bits.
    pub fn publish_exti_internal_levels(&self, gpio_flags: u32) {
        let flags = gpio_flags | self.exti_internal_flags.load(Ordering::SeqCst);
        let active = flags & self.exti_intenr.load(Ordering::SeqCst);
        self.set_level(AWU_IRQ, active & (1 << EXTI_AWU_LINE) != 0);
    }

    pub fn priority_of(&self, irq: u32) -> u8 {
        let word = self.priority[(irq / 4) as usize % 64].load(Ordering::Relaxed);
        (word >> ((irq % 4) * 8)) as u8
    }

    fn requested(&self, word: usize) -> u32 {
        self.pending[word].load(Ordering::SeqCst) | self.level[word].load(Ordering::SeqCst)
    }

    /// The interrupt the core should take next, if any: an enabled, requested,
    /// not already active interrupt that passes the threshold, lowest IPRIOR
    /// value first and the lowest number among equals.
    pub fn next_interrupt(&self) -> Option<u32> {
        let threshold = (self.threshold.load(Ordering::Relaxed) as u8) & PRIORITY_MASK;
        let mut best: Option<(u8, u32)> = None;
        for word in 0..WORDS {
            let mut ready = self.requested(word)
                & self.enabled[word].load(Ordering::SeqCst)
                & !self.active[word].load(Ordering::SeqCst);
            while ready != 0 {
                let irq = word as u32 * 32 + ready.trailing_zeros();
                ready &= ready - 1;
                let prio = self.priority_of(irq);
                // A non-zero threshold masks priorities at or below it.
                if threshold != 0 && prio >= threshold {
                    continue;
                }
                if best.is_none_or(|(p, _)| prio < p) {
                    best = Some((prio, irq));
                }
            }
        }
        best.map(|(_, irq)| irq)
    }

    /// Interrupt entry: the latched pend is consumed and the interrupt
    /// becomes active. A level source keeps requesting until it deasserts.
    pub fn acknowledge(&self, irq: u32) {
        if let Some((word, bit)) = split(irq) {
            self.pending[word].fetch_and(!bit, Ordering::SeqCst);
            self.active[word].fetch_or(bit, Ordering::SeqCst);
        }
    }

    /// `mret` from the handler of `irq`.
    pub fn complete(&self, irq: u32) {
        if let Some((word, bit)) = split(irq) {
            self.active[word].fetch_and(!bit, Ordering::SeqCst);
        }
    }

    /// A VTF channel bound to `irq` replaces the vector-table lookup with its
    /// programmed handler address.
    pub fn vtf_target(&self, irq: u32) -> Option<u32> {
        let ids = self.vtf_id.load(Ordering::Relaxed);
        (0..VTF_CHANNELS).find_map(|ch| {
            let addr = self.vtf_addr[ch].load(Ordering::Relaxed);
            ((ids >> (ch * 8)) & 0xFF == irq && addr & 1 != 0).then_some(addr & !1)
        })
    }

    fn active_count(&self) -> u32 {
        self.active
            .iter()
            .map(|w| w.load(Ordering::Relaxed).count_ones())
            .sum()
    }

    fn read_reg(&self, offset: u64) -> u32 {
        let word = ((offset & 0x1F) / 4) as usize;
        match offset {
            0x000..=0x01F => self.enabled[word].load(Ordering::SeqCst),
            0x020..=0x03F => self.requested(word),
            0x040 => self.threshold.load(Ordering::Relaxed),
            0x04C => {
                let depth = self.active_count().min(2);
                let nest = (1u32 << depth) - 1;
                let any_active = u32::from(depth != 0) << 8;
                let any_pending = u32::from((0..WORDS).any(|w| self.requested(w) != 0)) << 9;
                nest | any_active | any_pending
            }
            0x050 => self.vtf_id.load(Ordering::Relaxed),
            0x060 | 0x064 => self.vtf_addr[((offset - 0x060) / 4) as usize].load(Ordering::Relaxed),
            0x300..=0x31F => self.active[word].load(Ordering::SeqCst),
            0x400..=0x4FF => self.priority[((offset - 0x400) / 4) as usize].load(Ordering::Relaxed),
            0xD10 => self.sctlr.load(Ordering::Relaxed),
            _ => 0,
        }
    }

    fn write_reg(&self, offset: u64, value: u32) {
        let word = ((offset & 0x1F) / 4) as usize;
        match offset {
            0x040 => self
                .threshold
                .store(value & u32::from(PRIORITY_MASK), Ordering::Relaxed),
            0x050 => self.vtf_id.store(value & 0xFFFF, Ordering::Relaxed),
            0x060 | 0x064 => {
                self.vtf_addr[((offset - 0x060) / 4) as usize].store(value, Ordering::Relaxed)
            }
            0x100..=0x11F => {
                self.enabled[word].fetch_or(value, Ordering::SeqCst);
            }
            0x180..=0x19F => {
                self.enabled[word].fetch_and(!value, Ordering::SeqCst);
            }
            0x200..=0x21F => {
                self.pending[word].fetch_or(value, Ordering::SeqCst);
            }
            0x280..=0x29F => {
                self.pending[word].fetch_and(!value, Ordering::SeqCst);
            }
            0x400..=0x4FF => {
                let mask = u32::from_ne_bytes([PRIORITY_MASK; 4]);
                self.priority[((offset - 0x400) / 4) as usize]
                    .store(value & mask, Ordering::Relaxed)
            }
            0xD10 => {
                if value & SCTLR_SETEVENT != 0 {
                    self.raise_event();
                }
                self.sctlr.store(value, Ordering::Relaxed)
            }
            _ => crate::census_reg!("qingke_pfic:QingkePfic", offset, "write"),
        }
    }

    /// An MMIO store from a unit test that holds only the shared state.
    #[cfg(test)]
    pub fn write_reg_for_test(&self, offset: u64, value: u32) {
        self.write_reg(offset, value);
    }

    fn is_write_one_bank(offset: u64) -> bool {
        (0x100..0x2A0).contains(&offset)
    }
}

/// MMIO view of [`QingkePficState`].
#[derive(Debug, Clone, Default)]
pub struct QingkePfic {
    pub state: Arc<QingkePficState>,
}

impl QingkePfic {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PficSnapshot {
    enabled: Vec<u32>,
    pending: Vec<u32>,
    active: Vec<u32>,
    priority: Vec<u32>,
    threshold: u32,
    vtf_id: u32,
    vtf_addr: Vec<u32>,
    sctlr: u32,
    event: bool,
    exti_internal_flags: u32,
}

fn load_all(words: &[AtomicU32]) -> Vec<u32> {
    words.iter().map(|w| w.load(Ordering::Relaxed)).collect()
}

fn store_all(words: &[AtomicU32], values: &[u32]) {
    for (w, v) in words.iter().zip(values) {
        w.store(*v, Ordering::Relaxed);
    }
}

impl QingkePfic {
    fn capture(&self) -> PficSnapshot {
        let s = &self.state;
        PficSnapshot {
            enabled: load_all(&s.enabled),
            pending: load_all(&s.pending),
            active: load_all(&s.active),
            priority: load_all(&s.priority),
            threshold: s.threshold.load(Ordering::Relaxed),
            vtf_id: s.vtf_id.load(Ordering::Relaxed),
            vtf_addr: load_all(&s.vtf_addr),
            sctlr: s.sctlr.load(Ordering::Relaxed),
            event: s.event.load(Ordering::Relaxed),
            exti_internal_flags: s.exti_internal_flags.load(Ordering::Relaxed),
        }
    }

    /// Level lines are not restored: their sources re-drive them.
    fn apply(&self, snap: &PficSnapshot) {
        let s = &self.state;
        store_all(&s.enabled, &snap.enabled);
        store_all(&s.pending, &snap.pending);
        store_all(&s.active, &snap.active);
        store_all(&s.priority, &snap.priority);
        s.threshold.store(snap.threshold, Ordering::Relaxed);
        s.vtf_id.store(snap.vtf_id, Ordering::Relaxed);
        store_all(&s.vtf_addr, &snap.vtf_addr);
        s.sctlr.store(snap.sctlr, Ordering::Relaxed);
        s.event.store(snap.event, Ordering::Relaxed);
        s.exti_internal_flags
            .store(snap.exti_internal_flags, Ordering::Relaxed);
    }
}

impl Peripheral for QingkePfic {
    // A register bank over shared state; the core does the arbitration.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.state.read_reg(offset & !3);
        Ok((word >> ((offset % 4) * 8)) as u8)
    }

    /// Byte stores. The write-1 banks act on the byte's bits alone, so a
    /// `strb` to IENR cannot disturb its neighbours; IPRIOR is byte-addressed
    /// on silicon (`NVIC_SetPriority` stores one byte).
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        if QingkePficState::is_write_one_bank(reg) {
            self.state.write_reg(reg, u32::from(value) << shift);
        } else {
            let old = self.state.read_reg(reg);
            let merged = (old & !(0xFF << shift)) | (u32::from(value) << shift);
            self.state.write_reg(reg, merged);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.state.write_reg(offset, value);
            return Ok(());
        }
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + i as u64, *byte)?;
        }
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self.capture()).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let snap: PficSnapshot = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("QingkePfic snapshot decode: {error}"))
        })?;
        self.apply(&snap);
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&self.capture()).expect("bincode serialize QingkePfic")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let snap: PficSnapshot = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "QingkePfic runtime snapshot decode: {error}"
            ))
        })?;
        self.apply(&snap);
        Ok(())
    }

    fn qingke_pfic_state(&self) -> Option<Arc<QingkePficState>> {
        Some(self.state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTICK: u32 = 12;
    const SW: u32 = 14;
    const USART1: u32 = 32;

    #[test]
    fn enable_bank_uses_the_interrupt_number_as_the_bit() {
        let mut p = QingkePfic::new();
        // NVIC_EnableIRQ(USART1_IRQn): IENR[32 >> 5] = 1 << (32 & 0x1F).
        p.write_u32(0x104, 1).unwrap();
        p.write_u32(0x100, 1 << SYSTICK).unwrap();
        assert_eq!(p.read_u32(0x000).unwrap(), 1 << SYSTICK);
        assert_eq!(p.read_u32(0x004).unwrap(), 1);
        assert_eq!(p.read_u32(0x104).unwrap(), 0, "IENR is write-only");
        p.write_u32(0x180, 1 << SYSTICK).unwrap();
        assert_eq!(p.read_u32(0x000).unwrap(), 0);
    }

    #[test]
    fn nothing_is_taken_until_enabled_and_requested() {
        let mut p = QingkePfic::new();
        p.state.pend(USART1);
        assert_eq!(p.state.next_interrupt(), None, "pending but disabled");
        p.write_u32(0x104, 1).unwrap();
        assert_eq!(p.state.next_interrupt(), Some(USART1));
        assert_eq!(p.read_u32(0x024).unwrap(), 1, "IPR1 shows the pend");
        p.write_u32(0x284, 1).unwrap();
        assert_eq!(p.state.next_interrupt(), None, "IPRR cleared it");
    }

    #[test]
    fn entry_consumes_a_latched_pend_but_not_a_level() {
        let mut p = QingkePfic::new();
        p.write_u32(0x100, (1 << SYSTICK) | (1 << SW)).unwrap();
        p.write_u32(0x200, 1 << SW).unwrap();
        p.state.set_level(SYSTICK, true);

        // Equal priority: the lower number wins.
        assert_eq!(p.state.next_interrupt(), Some(SYSTICK));
        p.state.acknowledge(SYSTICK);
        assert_eq!(p.read_u32(0x300).unwrap(), 1 << SYSTICK, "IACTR");
        assert_eq!(
            p.read_u32(0x04C).unwrap() & 0x1FF,
            0x101,
            "one level active"
        );
        // Active interrupts are not re-taken; the next candidate is SW.
        assert_eq!(p.state.next_interrupt(), Some(SW));
        p.state.complete(SYSTICK);
        // Still level-asserted: requested again after mret.
        assert_eq!(p.state.next_interrupt(), Some(SYSTICK));
        p.state.set_level(SYSTICK, false);
        p.state.acknowledge(SW);
        p.state.complete(SW);
        assert_eq!(p.state.next_interrupt(), None, "SW pend consumed on entry");
    }

    #[test]
    fn priority_and_threshold_use_bits_7_6() {
        let mut p = QingkePfic::new();
        p.write_u32(0x100, (1 << SYSTICK) | (1 << SW)).unwrap();
        // NVIC_SetPriority is a byte store; the low six bits are not kept.
        p.write(0x400 + u64::from(SYSTICK), 0xFF).unwrap();
        p.write(0x400 + u64::from(SW), 0x40).unwrap();
        assert_eq!(p.state.priority_of(SYSTICK), 0xC0);
        p.state.set_level(SYSTICK, true);
        p.state.pend(SW);
        assert_eq!(p.state.next_interrupt(), Some(SW), "0x40 beats 0xC0");
        p.write_u32(0x040, 0x40).unwrap();
        assert_eq!(p.state.next_interrupt(), None, "both at or below threshold");
        p.write_u32(0x040, 0x80).unwrap();
        assert_eq!(p.state.next_interrupt(), Some(SW));
    }

    #[test]
    fn vtf_channel_needs_its_enable_bit() {
        let mut p = QingkePfic::new();
        p.write_u32(0x050, (USART1 << 8) | SYSTICK).unwrap();
        p.write_u32(0x060, 0x0000_0200).unwrap();
        p.write_u32(0x064, 0x0000_0301).unwrap();
        assert_eq!(p.state.vtf_target(SYSTICK), None, "channel 0 disabled");
        assert_eq!(p.state.vtf_target(USART1), Some(0x300));
    }

    #[test]
    fn snapshot_round_trips() {
        let mut p = QingkePfic::new();
        p.write_u32(0x100, 1 << SYSTICK).unwrap();
        p.write_u32(0x200, 1 << SW).unwrap();
        let bytes = p.runtime_snapshot();
        let mut q = QingkePfic::new();
        q.restore_runtime_snapshot(&bytes).unwrap();
        assert_eq!(q.read_u32(0x000).unwrap(), 1 << SYSTICK);
        assert_eq!(q.read_u32(0x020).unwrap(), 1 << SW);
    }
}
