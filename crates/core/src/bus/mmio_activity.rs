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
    ///
    /// Also the one place the shared [`crate::CycleClock`] is refreshed from
    /// `current_cycle` (issue #842). Every CPU-facing peripheral access —
    /// all six `dev.read*` dispatch sites and all three `dev.write*` ones —
    /// passes through here first, which is exactly the property the read-side
    /// freshness fix needs and exactly the property the bug lacked: a sync
    /// hung off individual accessors is a sync that some accessor will be
    /// added without.
    ///
    /// It lives HERE rather than in the CPU batch loop because the loop runs
    /// per retired instruction and this runs per MMIO. The batch loop keeps
    /// `current_cycle` live with a single in-place add; paying the ATOMIC store
    /// only when a peripheral is actually touched is what keeps the fix inside
    /// the throughput gate (the ALU spin fixture it measures does almost no
    /// MMIO, and firmware that polls a counter pays it once per poll).
    #[inline]
    pub(crate) fn note_mmio_activity(&self, peri_idx: usize, offset: u64) {
        // Before the bounds check: a model that lazily advances off the clock
        // must see "now" even if the index lookup below bails.
        //
        // Feature-gated because lazy advance is only reachable under it —
        // `legacy_tick_index_active` keeps every model on the per-cycle walk
        // when the flag is off, and the batch loop's cycle accumulator is
        // gated the same way. So a non-`event-scheduler` build has no reader
        // for a mid-boundary clock value, and stays byte-identical.
        #[cfg(feature = "event-scheduler")]
        self.cycle_clock.publish(self.current_cycle);
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
