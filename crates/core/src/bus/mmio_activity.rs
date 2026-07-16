// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Batch-local MMIO activity bookkeeping for idle/timer-poll coalesce.
//!
//! **CPU-agnostic:** counters only see [`crate::MmioAccessClass`] from each
//! peripheral. Chip register maps live on peripheral models (e.g. SYSTIMER).

use super::SystemBus;

impl SystemBus {
    /// Clear batch-local MMIO activity counters (call before each CPU batch).
    #[inline]
    pub fn reset_mmio_activity_counters(&self) {
        self.freerunning_timer_poll_mmio.set(0);
        self.side_effecting_mmio.set(0);
    }

    /// True when the just-finished batch only performed freerunning-timer
    /// polls (no side-effecting MMIO). Consumes and clears the counters.
    /// Chip-specific which regs count as polls — decided by each peripheral.
    #[inline]
    pub fn take_timer_poll_coalesce_eligible(&self) -> bool {
        let timer = self.freerunning_timer_poll_mmio.replace(0);
        let side = self.side_effecting_mmio.replace(0);
        // At least two poll accesses (e.g. OP update + value read).
        timer >= 2 && side == 0
    }

    /// Bookkeep one peripheral MMIO via [`Peripheral::mmio_access_class`]
    /// only — no chip name or register map knowledge on the bus.
    #[inline]
    pub(crate) fn note_mmio_activity(&self, peri_idx: usize, offset: u64) {
        let Some(p) = self.peripherals.get(peri_idx) else {
            return;
        };
        match p.dev.mmio_access_class(offset) {
            crate::MmioAccessClass::FreerunningTimerPoll => {
                self.freerunning_timer_poll_mmio
                    .set(self.freerunning_timer_poll_mmio.get().saturating_add(1));
            }
            crate::MmioAccessClass::SideEffecting => {
                self.side_effecting_mmio
                    .set(self.side_effecting_mmio.get().saturating_add(1));
            }
            crate::MmioAccessClass::SideEffectFree => {}
        }
    }
}
