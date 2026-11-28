// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, SimResult};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Shared state for NVIC registers.
#[derive(Debug)]
pub struct NvicState {
    /// Local Cortex-M event latch and SCR.SEVONPEND mirror. Disabled IRQs
    /// still generate events when they transition to pending with SEVONPEND.
    pub event_register: AtomicBool,
    pub sev_on_pend: AtomicBool,
    /// Up to 496 external IRQs, as allowed by ARMv7-M/ARMv8-M.
    pub iser: [AtomicU32; 16],
    /// Cached "any ISER bit is set" so the per-boundary interrupt collection
    /// can skip its sixteen-word scan on a machine that never enabled one.
    pub any_enabled: AtomicBool,
    pub ispr: [AtomicU32; 16],
    pub iabr: [AtomicU32; 16],
    pub ipr: [AtomicU32; 240], // Priority registers (simplified)
    /// Which ISPR bits were set by an asserted LEVEL source (a peripheral
    /// whose `irq_line_level()` is `Some(..)`). The bus drops them when that
    /// line deasserts; clearing a still-asserted source through ICPR immediately
    /// re-pends it. On silicon a level-sensitive interrupt's pending state
    /// follows the line: firmware that clears the status flag inside the
    /// handler must not be re-entered for the same event. Bits set by a
    /// software ISPR write are NOT marked here and are never auto-cleared — a
    /// software pend of a low line is taken once, as on hardware.
    pub level_pended: [AtomicU32; 16],
}

impl NvicState {
    pub fn pend(&self, word: usize, mask: u32) {
        let previous = self.ispr[word].fetch_or(mask, Ordering::SeqCst);
        if mask & !previous != 0 && self.sev_on_pend.load(Ordering::Relaxed) {
            self.event_register.store(true, Ordering::Relaxed);
        }
    }

    /// Clear pending bits, then immediately restore any bit whose level source
    /// remains asserted. The `level_pended` marker is cleared separately when
    /// the bus observes that source deasserting.
    pub fn clear_pending(&self, word: usize, mask: u32) {
        if word >= self.ispr.len() {
            return;
        }
        self.ispr[word].fetch_and(!mask, Ordering::SeqCst);
        let active_levels = self.level_pended[word].load(Ordering::SeqCst) & mask;
        if active_levels != 0 {
            self.pend(word, active_levels);
        }
    }

    /// Read the configured priority byte for an external IRQ
    /// (`irq` is 0-based — exception_number minus 16).
    /// Used by CortexM::exception_priority for IRQs ≥ 16.
    pub fn ipr_priority(&self, irq: usize) -> u8 {
        let reg = irq / 4;
        let byte = irq % 4;
        if reg < self.ipr.len() {
            ((self.ipr[reg].load(Ordering::Relaxed) >> (byte * 8)) & 0xFF) as u8
        } else {
            0xFF
        }
    }
}

impl Default for NvicState {
    fn default() -> Self {
        Self {
            event_register: AtomicBool::new(false),
            sev_on_pend: AtomicBool::new(false),
            iser: Default::default(),
            any_enabled: AtomicBool::new(false),
            ispr: Default::default(),
            iabr: Default::default(),
            ipr: [0; 240].map(|_| AtomicU32::new(0)),
            level_pended: Default::default(),
        }
    }
}

/// Nested Vectored Interrupt Controller (NVIC) mock.
#[derive(Debug, Clone)]
pub struct Nvic {
    pub state: Arc<NvicState>,
}

impl Nvic {
    pub fn new(state: Arc<NvicState>) -> Self {
        Self { state }
    }

    pub fn is_enabled(&self, irq: u32) -> bool {
        if irq < 16 {
            return true;
        }
        let idx = ((irq - 16) / 32) as usize;
        let bit = (irq - 16) % 32;
        if idx < self.state.iser.len() {
            (self.state.iser[idx].load(Ordering::SeqCst) & (1 << bit)) != 0
        } else {
            false
        }
    }

    pub fn acknowledge_interrupt(&self, irq: u32) {
        if irq >= 16 {
            let idx = ((irq - 16) / 32) as usize;
            let bit = (irq - 16) % 32;
            if idx < self.state.ispr.len() {
                // Clear pending, set active. Entry does not re-pend a level
                // source; see `SystemBus::clear_nvic_pending`.
                self.state.ispr[idx].fetch_and(!(1 << bit), Ordering::SeqCst);
                self.state.iabr[idx].fetch_or(1 << bit, Ordering::SeqCst);
            }
        }
    }

    pub fn complete_interrupt(&self, irq: u32) {
        if irq >= 16 {
            let idx = ((irq - 16) / 32) as usize;
            let bit = (irq - 16) % 32;
            if idx < self.state.iabr.len() {
                // Clear active
                self.state.iabr[idx].fetch_and(!(1 << bit), Ordering::SeqCst);
            }
        }
    }
}

impl Peripheral for Nvic {
    // Inert walk: enable/pending register bank; the NVIC pend+enable scan runs in the bus tick loop over shared state, not this peripheral's tick() (the trait-default no-op).
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    // The NVIC's own `tick()` is the trait-default no-op (see above), so it is
    // never "active" for the per-cycle walk. Report that explicitly: the
    // default `legacy_tick_active() == true` kept this inert register bank in
    // the walk set AND — because idle fast-forward's `legacy_safe` gate keys on
    // `legacy_tick_active` — spuriously blocked idle FF on EVERY Cortex-M board.
    // Overriding to `false` is byte-identical (removing a no-op tick changes
    // nothing) and consistent with `needs_legacy_walk() == false` above.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_idx = (offset / 4) as usize;
        let byte_offset = (offset % 4) as usize;

        // ISER and ICER share a read view, as do ISPR and ICPR: a read of the
        // clear-enable / clear-pending register returns the current enable /
        // pending mask (ARMv7-M ARM DDI 0403E, B3.4.3). Firmware that saves
        // the enable state from ICER, writes a mask, then restores through
        // ISER (FreeRTOS tickless idle) depends on this. A read-as-zero ICER
        // made that save 0, so the restore never re-enabled IRQ 17.
        let val = if offset < 0x40 {
            // ISER0-15
            self.state.iser[reg_idx].load(Ordering::SeqCst)
        } else if (0x80..0xC0).contains(&offset) {
            // ICER0-15: same read value as ISER
            self.state.iser[((offset - 0x80) / 4) as usize].load(Ordering::SeqCst)
        } else if (0x100..0x140).contains(&offset) {
            // ISPR0-15
            let real_idx = (offset - 0x100) / 4;
            self.state.ispr[real_idx as usize].load(Ordering::SeqCst)
        } else if (0x180..0x1C0).contains(&offset) {
            // ICPR0-15: same read value as ISPR
            self.state.ispr[((offset - 0x180) / 4) as usize].load(Ordering::SeqCst)
        } else if (0x200..0x240).contains(&offset) {
            // IABR0-15
            let real_idx = (offset - 0x200) / 4;
            self.state.iabr[real_idx as usize].load(Ordering::SeqCst)
        } else if (0x300..0x6BC).contains(&offset) {
            // IPR0-239
            let real_idx = (offset - 0x300) / 4;
            self.state.ipr[real_idx as usize].load(Ordering::SeqCst)
        } else {
            0
        };

        Ok(((val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_idx = (offset / 4) as usize;
        let byte_offset = (offset % 4) as usize;
        let mask = (value as u32) << (byte_offset * 8);

        if offset < 0x40 {
            // ISER: Writing 1 sets the enable bit
            self.state.iser[reg_idx].fetch_or(mask, Ordering::SeqCst);
            if mask != 0 {
                self.state.any_enabled.store(true, Ordering::Relaxed);
            }
        } else if (0x80..0xC0).contains(&offset) {
            // ICER: Writing 1 clears the enable bit. Recompute the cache only
            // when the write actually changed the word.
            let real_idx = reg_idx - 0x80 / 4;
            let before = self.state.iser[real_idx].fetch_and(!mask, Ordering::SeqCst);
            if before & mask != 0
                && self
                    .state
                    .iser
                    .iter()
                    .all(|word| word.load(Ordering::Relaxed) == 0)
            {
                self.state.any_enabled.store(false, Ordering::Relaxed);
            }
        } else if (0x100..0x140).contains(&offset) {
            // ISPR: Writing 1 sets the pending bit
            let real_idx = reg_idx - 0x100 / 4;
            self.state.pend(real_idx, mask);
        } else if (0x180..0x1C0).contains(&offset) {
            // ICPR: Writing 1 clears edge/software pending bits. An asserted
            // level source immediately re-pends, as it does on the NVIC.
            let real_idx = reg_idx - 0x180 / 4;
            self.state.clear_pending(real_idx, mask);
        } else if (0x300..0x6BC).contains(&offset) {
            // IPR: Priority registers
            let real_idx = (offset - 0x300) / 4;
            let mut old_val = self.state.ipr[real_idx as usize].load(Ordering::SeqCst);
            loop {
                let mut new_val = old_val;
                let m = 0xFF << (byte_offset * 8);
                new_val &= !m;
                new_val |= mask;
                match self.state.ipr[real_idx as usize].compare_exchange_weak(
                    old_val,
                    new_val,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => old_val = actual,
                }
            }
        }

        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        let iser: Vec<u32> = self
            .state
            .iser
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .collect();
        let ispr: Vec<u32> = self
            .state
            .ispr
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .collect();
        serde_json::json!({
            "iser": iser,
            "ispr": ispr,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icpr_re_pends_an_asserted_level_source() {
        let state = Arc::new(NvicState::default());
        let mut nvic = Nvic::new(Arc::clone(&state));
        let mask = 1 << 6;
        state.pend(0, mask);
        state.level_pended[0].store(mask, Ordering::SeqCst);

        nvic.write(0x180, mask as u8).unwrap();

        assert_eq!(state.ispr[0].load(Ordering::SeqCst) & mask, mask);
        assert_eq!(state.level_pended[0].load(Ordering::SeqCst) & mask, mask);
    }

    /// Exception entry clears pending even while the level source is still
    /// asserted: the handler has not cleared the flag yet. Re-pending here
    /// re-entered a USART RX handler 102,067 times in the uart-chain world.
    #[test]
    fn entry_clears_pending_of_an_asserted_level_source() {
        let state = Arc::new(NvicState::default());
        let nvic = Nvic::new(Arc::clone(&state));
        let mask = 1 << 5;
        state.pend(0, mask);
        state.level_pended[0].store(mask, Ordering::SeqCst);

        nvic.acknowledge_interrupt(16 + 5);

        assert_eq!(state.ispr[0].load(Ordering::SeqCst) & mask, 0);
        assert_eq!(state.iabr[0].load(Ordering::SeqCst) & mask, mask);
    }

    #[test]
    fn icpr_still_clears_software_pending_bits() {
        let state = Arc::new(NvicState::default());
        let mut nvic = Nvic::new(Arc::clone(&state));
        let mask = 1 << 7;
        state.pend(0, mask);

        nvic.write(0x180, mask as u8).unwrap();

        assert_eq!(state.ispr[0].load(Ordering::SeqCst) & mask, 0);
        assert_eq!(state.level_pended[0].load(Ordering::SeqCst) & mask, 0);
    }

    #[test]
    fn writing_iser_sets_irq_17_and_icer_reads_the_same_mask() {
        let state = Arc::new(NvicState::default());
        let mut nvic = Nvic::new(Arc::clone(&state));
        let irq17 = 1u32 << 17;
        nvic.write_u32(0, irq17).unwrap();
        assert_eq!(nvic.read_u32(0).unwrap(), irq17);
        assert_eq!(
            nvic.read_u32(0x80).unwrap(),
            irq17,
            "ICER must read back the enable state, not zero"
        );
    }

    #[test]
    fn icer_write_clears_irq_17_after_a_save_of_its_read_value() {
        // The FreeRTOS tickless-idle sequence: load ICER, AND with a mask,
        // store the mask to ICER (disabling those bits), then later restore
        // the saved bits through ISER.
        let state = Arc::new(NvicState::default());
        let mut nvic = Nvic::new(Arc::clone(&state));
        let irq17 = 1u32 << 17;
        let irq0 = 1u32;
        nvic.write_u32(0, irq17 | irq0).unwrap();
        let mask = 0xBDFF_06FC;
        let saved = nvic.read_u32(0x80).unwrap() & mask;
        assert_eq!(saved, irq17, "IRQ 17 is in the mask and must be saved");
        nvic.write_u32(0x80, mask).unwrap();
        assert_eq!(
            nvic.read_u32(0).unwrap() & irq17,
            0,
            "ICER write must clear IRQ 17"
        );
        assert_eq!(
            nvic.read_u32(0).unwrap() & irq0,
            irq0,
            "IRQ 0 is outside the mask"
        );
        nvic.write_u32(0, saved).unwrap();
        assert_eq!(nvic.read_u32(0).unwrap() & irq17, irq17);
        assert_eq!(nvic.read_u32(0).unwrap() & irq0, irq0);
    }

    #[test]
    fn icpr_reads_back_the_pending_mask() {
        let state = Arc::new(NvicState::default());
        let nvic = Nvic::new(Arc::clone(&state));
        let irq17 = 1u32 << 17;
        state.pend(0, irq17);
        assert_eq!(nvic.read_u32(0x100).unwrap(), irq17);
        assert_eq!(nvic.read_u32(0x180).unwrap(), irq17);
    }

    #[test]
    fn irq_264_uses_the_ninth_nvic_word_for_enable_pending_and_active_state() {
        let state = Arc::new(NvicState::default());
        let mut nvic = Nvic::new(Arc::clone(&state));
        let irq = 264u32;
        let word = (irq / 32) as usize;
        let bit = 1 << (irq % 32);

        nvic.write_u32((word * 4) as u64, bit).unwrap();
        assert!(nvic.is_enabled(16 + irq));
        assert_eq!(nvic.read_u32((word * 4) as u64).unwrap(), bit);

        nvic.write_u32(0x100 + (word * 4) as u64, bit).unwrap();
        assert_eq!(nvic.read_u32(0x100 + (word * 4) as u64).unwrap(), bit);

        nvic.acknowledge_interrupt(16 + irq);
        assert_eq!(state.ispr[word].load(Ordering::SeqCst) & bit, 0);
        assert_eq!(state.iabr[word].load(Ordering::SeqCst) & bit, bit);
        nvic.complete_interrupt(16 + irq);
        assert_eq!(state.iabr[word].load(Ordering::SeqCst) & bit, 0);
    }
}
