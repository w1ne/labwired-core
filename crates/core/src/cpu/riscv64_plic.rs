// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! SiFive-style platform-level interrupt controller (RISC-V PLIC spec 1.0.0),
//! as seen by the one hart a [`super::RiscV64`] runs.
//!
//! | Offset | Register | Access |
//! | --- | --- | --- |
//! | 0x00_0000 + 4·id | source `id` priority (id 0 reads 0) | R/W, WARL to `priority_bits` |
//! | 0x00_1000 + 4·w | pending bits, sources 32w..32w+31 | R |
//! | 0x00_2000 + 0x80·ctx + 4·w | context `ctx` enable bits | R/W |
//! | 0x20_0000 + 0x1000·ctx | context `ctx` priority threshold | R/W, WARL |
//! | 0x20_0004 + 0x1000·ctx | context `ctx` claim (read) / complete (write) | R/W |
//!
//! Every context's enables and threshold are stored, and any context can
//! claim and complete, but only `context` (this hart's M-mode context) drives
//! an interrupt line: `MEIP` is set while some pending, enabled source has a
//! priority above that context's threshold. Priority 0 never interrupts and is
//! never claimed. A claim returns the highest-priority pending source enabled
//! for the context with priority above its threshold (ties go to the lowest ID),
//! clears its pending bit and holds the source in service
//! until the matching complete, which is ignored unless the source is enabled
//! for that context. While a source is in service its gateway forwards
//! nothing; a level source still asserted after the complete re-pends on its
//! next assertion. Sources are pended by [`Plic::raise`] (the bus's peripheral
//! IRQ number, which is the PLIC source ID); de-assertion is not seen, so a
//! level source that drops before the claim stays pending.
//!
//! Not modelled: the other harts' contexts as interrupt targets, S-mode
//! `SEIP`, and PolarFire's per-pin GPIO direct sources (13-50).

/// Register map geometry of one PLIC instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlicParams {
    pub base: u64,
    /// Source IDs are 1..`num_sources`; ID 0 means "no interrupt".
    pub num_sources: u32,
    pub num_contexts: u32,
    /// Width of the priority and threshold fields.
    pub priority_bits: u32,
    /// The context whose notification drives this hart's `MEIP`.
    pub context: u32,
}

pub(crate) const PLIC_SIZE: u64 = 0x400_0000;
const PENDING: u64 = 0x1000;
const ENABLE: u64 = 0x2000;
const ENABLE_STRIDE: u64 = 0x80;
const CONTEXT: u64 = 0x20_0000;
const CONTEXT_STRIDE: u64 = 0x1000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Plic {
    pub base: u64,
    pub num_sources: u32,
    pub priority_mask: u32,
    pub context: u32,
    /// Per source ID (index 0 unused).
    pub priority: Vec<u32>,
    pub pending: Vec<u32>,
    /// Claimed and not yet completed, per source (the gateway is closed).
    pub in_service: Vec<u32>,
    /// `enable[ctx]` holds that context's enable words.
    pub enable: Vec<Vec<u32>>,
    pub threshold: Vec<u32>,
}

fn bit(words: &[u32], id: u32) -> bool {
    words
        .get((id / 32) as usize)
        .is_some_and(|w| w & (1 << (id % 32)) != 0)
}

fn set_bit(words: &mut [u32], id: u32, on: bool) {
    if let Some(w) = words.get_mut((id / 32) as usize) {
        if on {
            *w |= 1 << (id % 32);
        } else {
            *w &= !(1 << (id % 32));
        }
    }
}

impl Plic {
    pub fn new(p: PlicParams) -> Self {
        let words = p.num_sources.div_ceil(32) as usize;
        Self {
            base: p.base,
            num_sources: p.num_sources,
            priority_mask: (1 << p.priority_bits) - 1,
            context: p.context,
            priority: vec![0; p.num_sources as usize],
            pending: vec![0; words],
            in_service: vec![0; words],
            enable: vec![vec![0; words]; p.num_contexts as usize],
            threshold: vec![0; p.num_contexts as usize],
        }
    }

    /// Back to the reset state, keeping the geometry.
    pub fn reset(&mut self) {
        self.priority.fill(0);
        self.pending.fill(0);
        self.in_service.fill(0);
        self.enable.iter_mut().for_each(|e| e.fill(0));
        self.threshold.fill(0);
    }

    pub(crate) fn offset(&self, addr: u64) -> Option<u64> {
        addr.checked_sub(self.base).filter(|off| *off < PLIC_SIZE)
    }

    fn valid(&self, id: u32) -> bool {
        id != 0 && id < self.num_sources
    }

    /// A peripheral asserted source `id`'s line.
    pub fn raise(&mut self, id: u32) {
        if self.valid(id) && !bit(&self.in_service, id) {
            set_bit(&mut self.pending, id, true);
        }
    }

    /// Highest-priority pending source enabled for `ctx` with priority above
    /// `floor` (lowest ID on a tie).
    fn best(&self, ctx: usize, floor: u32) -> Option<u32> {
        let enable = self.enable.get(ctx)?;
        let mut best: Option<(u32, u32)> = None;
        for id in 1..self.num_sources {
            if !bit(&self.pending, id) || !bit(enable, id) {
                continue;
            }
            let prio = self.priority[id as usize];
            if prio > floor && best.is_none_or(|(_, p)| prio > p) {
                best = Some((id, prio));
            }
        }
        best.map(|(id, _)| id)
    }

    /// The external-interrupt line into this hart's M-mode.
    pub fn meip(&self) -> bool {
        let ctx = self.context as usize;
        let floor = self.threshold.get(ctx).copied().unwrap_or(0);
        self.best(ctx, floor).is_some()
    }

    fn claim(&mut self, ctx: usize) -> u32 {
        let threshold = self.threshold.get(ctx).copied().unwrap_or(0);
        match self.best(ctx, threshold) {
            Some(id) => {
                set_bit(&mut self.pending, id, false);
                set_bit(&mut self.in_service, id, true);
                id
            }
            None => 0,
        }
    }

    fn complete(&mut self, ctx: usize, id: u32) {
        if self.valid(id) && self.enable.get(ctx).is_some_and(|e| bit(e, id)) {
            set_bit(&mut self.in_service, id, false);
        }
    }

    /// Context `ctx` and word index `w` for an enable-array offset.
    fn enable_slot(&self, off: u64) -> Option<(usize, usize)> {
        let rel = off.checked_sub(ENABLE)?;
        let (ctx, w) = (
            (rel / ENABLE_STRIDE) as usize,
            ((rel % ENABLE_STRIDE) / 4) as usize,
        );
        (off < CONTEXT && ctx < self.enable.len() && w < self.pending.len()).then_some((ctx, w))
    }

    /// Mask of the enable bits word `w` can hold (no ID 0, none past the last source).
    fn word_mask(&self, w: usize) -> u32 {
        let mut mask = 0;
        for b in 0..32 {
            if self.valid(w as u32 * 32 + b) {
                mask |= 1 << b;
            }
        }
        mask
    }

    /// One 32-bit register read. Claim has a side effect, hence `&mut`.
    pub fn read_u32(&mut self, off: u64) -> u32 {
        let off = off & !3;
        if off < PENDING {
            let id = (off / 4) as u32;
            return if self.valid(id) {
                self.priority[id as usize]
            } else {
                0
            };
        }
        if off < ENABLE {
            return self
                .pending
                .get(((off - PENDING) / 4) as usize)
                .copied()
                .unwrap_or(0);
        }
        if let Some((ctx, w)) = self.enable_slot(off) {
            return self.enable[ctx][w];
        }
        if let Some(rel) = off.checked_sub(CONTEXT) {
            let ctx = (rel / CONTEXT_STRIDE) as usize;
            if ctx < self.threshold.len() {
                match rel % CONTEXT_STRIDE {
                    0 => return self.threshold[ctx],
                    4 => return self.claim(ctx),
                    _ => {}
                }
            }
        }
        0
    }

    pub fn write_u32(&mut self, off: u64, value: u32) {
        let off = off & !3;
        if off < PENDING {
            let id = (off / 4) as u32;
            if self.valid(id) {
                self.priority[id as usize] = value & self.priority_mask;
            }
            return;
        }
        if off < ENABLE {
            return; // pending bits are read-only
        }
        if let Some((ctx, w)) = self.enable_slot(off) {
            self.enable[ctx][w] = value & self.word_mask(w);
            return;
        }
        if let Some(rel) = off.checked_sub(CONTEXT) {
            let ctx = (rel / CONTEXT_STRIDE) as usize;
            if ctx < self.threshold.len() {
                match rel % CONTEXT_STRIDE {
                    0 => self.threshold[ctx] = value & self.priority_mask,
                    4 => self.complete(ctx, value),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PolarFire SoC geometry, U54_1 M-mode context.
    fn mpfs() -> Plic {
        Plic::new(PlicParams {
            base: 0x0C00_0000,
            num_sources: 187,
            num_contexts: 9,
            priority_bits: 3,
            context: 1,
        })
    }

    fn enable(p: &mut Plic, ctx: u64, id: u32) {
        let off = ENABLE + ENABLE_STRIDE * ctx + 4 * u64::from(id / 32);
        let v = p.read_u32(off) | (1 << (id % 32));
        p.write_u32(off, v);
    }

    const CLAIM1: u64 = CONTEXT + CONTEXT_STRIDE + 4;
    const THRESH1: u64 = CONTEXT + CONTEXT_STRIDE;

    #[test]
    fn priority_is_warl_and_source_zero_is_hardwired() {
        let mut p = mpfs();
        p.write_u32(4 * 51, 0xFF);
        assert_eq!(p.read_u32(4 * 51), 7);
        p.write_u32(0, 5);
        assert_eq!(p.read_u32(0), 0);
        p.write_u32(4 * 187, 5); // past the last source
        assert_eq!(p.read_u32(4 * 187), 0);
    }

    #[test]
    fn enable_bits_mask_id_zero_and_missing_sources() {
        let mut p = mpfs();
        p.write_u32(ENABLE + ENABLE_STRIDE, u32::MAX);
        assert_eq!(p.read_u32(ENABLE + ENABLE_STRIDE), 0xFFFF_FFFE);
        // Word 5 covers 160..191; sources stop at 186.
        p.write_u32(ENABLE + ENABLE_STRIDE + 20, u32::MAX);
        assert_eq!(p.read_u32(ENABLE + ENABLE_STRIDE + 20), 0x07FF_FFFF);
    }

    #[test]
    fn claim_complete_cycle_and_meip() {
        let mut p = mpfs();
        p.write_u32(4 * 51, 2);
        enable(&mut p, 1, 51);
        assert!(!p.meip());
        assert_eq!(p.read_u32(CLAIM1), 0);
        p.raise(51);
        assert_eq!(p.read_u32(PENDING + 4), 1 << (51 - 32));
        assert!(p.meip());
        assert_eq!(p.read_u32(CLAIM1), 51);
        assert!(!p.meip());
        assert_eq!(p.read_u32(PENDING + 4), 0);
        // The gateway is closed until completion.
        p.raise(51);
        assert!(!p.meip());
        p.write_u32(CLAIM1, 51);
        p.raise(51);
        assert!(p.meip());
    }

    #[test]
    fn threshold_masks_notification_and_claim() {
        let mut p = mpfs();
        p.write_u32(4 * 52, 3);
        enable(&mut p, 1, 52);
        p.write_u32(THRESH1, 3);
        assert_eq!(p.read_u32(THRESH1), 3);
        p.raise(52);
        assert!(!p.meip());
        assert_eq!(p.read_u32(CLAIM1), 0);
        p.write_u32(THRESH1, 2);
        assert!(p.meip());
        assert_eq!(p.read_u32(CLAIM1), 52);
    }

    #[test]
    fn priority_zero_never_interrupts_and_highest_priority_wins() {
        let mut p = mpfs();
        for id in [51, 52, 53] {
            enable(&mut p, 1, id);
            p.raise(id);
        }
        assert!(!p.meip());
        assert_eq!(p.read_u32(CLAIM1), 0);
        p.write_u32(4 * 51, 1);
        p.write_u32(4 * 52, 4);
        p.write_u32(4 * 53, 4);
        assert_eq!(p.read_u32(CLAIM1), 52); // tie with 53: lower ID
        assert_eq!(p.read_u32(CLAIM1), 53);
        assert_eq!(p.read_u32(CLAIM1), 51);
    }

    #[test]
    fn other_contexts_neither_interrupt_nor_complete_unenabled_sources() {
        let mut p = mpfs();
        p.write_u32(4 * 51, 1);
        enable(&mut p, 2, 51); // U54_1 S-mode
        p.raise(51);
        assert!(!p.meip());
        assert_eq!(p.read_u32(CLAIM1), 0);
        assert_eq!(p.read_u32(CONTEXT + 2 * CONTEXT_STRIDE + 4), 51);
        // Completing from a context the source is not enabled for is ignored.
        p.write_u32(CLAIM1, 51);
        p.raise(51);
        assert_eq!(p.read_u32(PENDING + 4), 0);
        p.write_u32(CONTEXT + 2 * CONTEXT_STRIDE + 4, 51);
        p.raise(51);
        assert_eq!(p.read_u32(PENDING + 4), 1 << (51 - 32));
    }
}
