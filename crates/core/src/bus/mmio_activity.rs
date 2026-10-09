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

    /// `(memory_reads, memory_writes, peripheral_accesses)` — run-lifetime
    /// bus access counters for resource metrics (always-on, cheap `Cell`s).
    #[inline]
    pub fn access_counts(&self) -> (u64, u64, u64) {
        (
            self.memory_reads.get(),
            self.memory_writes.get(),
            self.peripheral_accesses.get(),
        )
    }

    /// The peripheral register the firmware has been accessing back to back,
    /// if any: `(peripheral name, register offset, consecutive accesses, cycle
    /// of the latest)`. A large count whose latest access is at the end of a
    /// run that hit its step budget is a firmware waiting on a status bit that
    /// never changed, and this names the register it waits on.
    pub fn mmio_poll_streak(&self) -> Option<(String, u64, u64, u64)> {
        let (idx, off, n, cycle) = self.poll_streak.get();
        let p = self.peripherals.get(idx as usize)?;
        (n > 0).then(|| (p.name.clone(), u64::from(off), n, cycle))
    }

    /// Snapshot and zero the run-lifetime access counters.
    #[inline]
    pub fn take_access_counts(&self) -> (u64, u64, u64) {
        (
            self.memory_reads.replace(0),
            self.memory_writes.replace(0),
            self.peripheral_accesses.replace(0),
        )
    }

    /// True when `[addr, addr + width)` provably falls in a hole between
    /// `extra_mem` windows, so the linear probe every accessor runs must miss.
    ///
    /// Two compares and a length check instead of one bounds test per window.
    /// See [`SystemBus::extra_mem_gap`] for why the answer is exact rather
    /// than a heuristic.
    #[inline(always)]
    pub(crate) fn extra_mem_surely_misses(&self, addr: u64, width: u64) -> bool {
        match self.extra_mem_gap.get() {
            Some((floor, ceil, len)) => {
                len == self.extra_mem.len() && addr >= floor && addr.saturating_add(width) <= ceil
            }
            None => false,
        }
    }

    /// Record the `extra_mem`-free hole around `addr` after a probe missed it.
    ///
    /// Floor is the greatest END among windows starting at or below `addr`,
    /// ceiling the least BASE among those starting above it. No window can
    /// cover a byte in `[floor, ceil)`: one starting at or below `addr` ends
    /// at or before `floor`, one starting above `addr` starts at or after
    /// `ceil`. A window that straddles `addr` (covers it but not the whole
    /// access) pushes the floor past `addr` and the guard below stores
    /// nothing — the same shape `find_peripheral_index` uses for `last_gap`.
    #[inline]
    pub(crate) fn note_extra_mem_miss(&self, addr: u64) {
        let mut floor = 0u64;
        let mut ceil = u64::MAX;
        for mem in &self.extra_mem {
            if mem.base_addr <= addr {
                floor = floor.max(mem.base_addr.saturating_add(mem.data.len() as u64));
            } else {
                ceil = ceil.min(mem.base_addr);
            }
        }
        if floor <= addr && addr < ceil {
            self.extra_mem_gap
                .set(Some((floor, ceil, self.extra_mem.len())));
        }
    }

    #[inline]
    pub(crate) fn note_memory_read(&self) {
        self.note_memory_reads(1);
    }

    #[inline]
    pub(crate) fn note_memory_reads(&self, count: u64) {
        self.memory_reads
            .set(self.memory_reads.get().wrapping_add(count));
    }

    #[inline]
    pub(crate) fn note_memory_write(&self) {
        self.note_memory_writes(1);
    }

    #[inline]
    pub(crate) fn note_memory_writes(&self, count: u64) {
        self.memory_writes
            .set(self.memory_writes.get().wrapping_add(count));
    }

    /// Account for additional accesses to a [`Peripheral::is_plain_memory`]
    /// window after a CPU coalescer has performed the final physical write.
    ///
    /// Xtensa IRAM/DRAM are represented by `RamPeripheral`s, so their normal
    /// bus path counts them as peripheral accesses (rather than
    /// `memory_writes`). A coalesced store loop performs one real bus write and
    /// uses this helper for the elided identical accesses, preserving resource
    /// metrics without repeating the virtual dispatch or `RefCell` borrow.
    #[inline]
    pub(crate) fn note_plain_memory_accesses(&self, count: u64) {
        self.peripheral_accesses
            .set(self.peripheral_accesses.get().wrapping_add(count));
        self.side_effecting_mmio.set(
            self.side_effecting_mmio
                .get()
                .saturating_add(count.min(u64::from(u32::MAX)) as u32),
        );
    }

    /// True when the bus's winning route for the whole range is an Xtensa
    /// fixed-size RAM window. Using the winning route matters when peripheral
    /// windows overlap: merely finding any broad RAM range would let a CPU
    /// coalescer bypass the narrower MMIO device the normal dispatcher picks.
    #[inline]
    pub(crate) fn is_plain_xtensa_ram_range(&self, addr: u64, width: u64) -> bool {
        let Some(end) = addr.checked_add(width) else {
            return false;
        };
        let Some(idx) = self.find_peripheral_index(addr) else {
            return false;
        };
        let p = &self.peripherals[idx];
        // `is_plain_memory` is the property the coalescer needs -- a device
        // that only stores and serves bytes -- and it is a capability, so no
        // downcast to the concrete RAM type (which the downcast ratchet counts).
        end <= p.base.saturating_add(p.size) && p.dev.is_plain_memory()
    }

    /// [`crate::Bus::commit_ram_store_spin`] for `SystemBus`. The refusals
    /// and their order are the ones the RISC-V coalescer made itself when it
    /// reached this bus by downcast: an armed ESP32-C3 PMS or any observer
    /// refuses first, then an address whose 4-byte store is not wholly inside
    /// the flat `ram` window. Only after every check passes is anything
    /// written, so a refusal leaves the bus exactly as it was.
    #[inline]
    pub(crate) fn commit_ram_store_spin(&mut self, addr: u32, value: u32, stores: u64) -> bool {
        if self.esp32c3_pms_armed() || !self.observers.is_empty() {
            return false;
        }
        let Some(ram_off) = u64::from(addr).checked_sub(self.ram.base_addr) else {
            return false;
        };
        let ram_off = ram_off as usize;
        if ram_off
            .checked_add(4)
            .is_none_or(|end| end > self.ram.data.len())
        {
            return false;
        }
        self.ram.data[ram_off..ram_off + 4].copy_from_slice(&value.to_le_bytes());
        self.note_memory_writes(stores);
        true
    }

    /// [`crate::Bus::commit_plain_memory_store_spin`] for `SystemBus`. The
    /// same checks, in the same order, the Xtensa coalescer made after its
    /// downcast: no observers, and the winning route for the 4-byte store is
    /// a plain-memory window. Then ONE real `write_u32`, which keeps routing
    /// and the final backing value, and the elided accesses are accounted.
    #[inline]
    pub(crate) fn commit_plain_memory_store_spin(
        &mut self,
        addr: u32,
        value: u32,
        elided: u64,
    ) -> crate::SimResult<bool> {
        if !self.observers.is_empty() || !self.is_plain_xtensa_ram_range(u64::from(addr), 4) {
            return Ok(false);
        }
        crate::Bus::write_u32(self, u64::from(addr), value)?;
        self.note_plain_memory_accesses(elided);
        Ok(true)
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
    ///
    /// Also increments the run-lifetime [`Self::peripheral_accesses`] counter
    /// (resource metrics P1) — every peri MMIO, regardless of access class.
    #[inline]
    pub(crate) fn note_mmio_activity(&self, peri_idx: usize, offset: u64) {
        self.note_mmio_activities(peri_idx, offset, 1);
    }

    /// Bulk form used only after a peripheral has explicitly promised that
    /// repeated reads are stable and side-effect-free until the next event.
    #[inline]
    pub(crate) fn note_mmio_activities(&self, peri_idx: usize, offset: u64, count: u32) {
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
        self.peripheral_accesses.set(
            self.peripheral_accesses
                .get()
                .wrapping_add(u64::from(count)),
        );
        let Some(p) = self.peripherals.get(peri_idx) else {
            return;
        };
        let class = p.dev.mmio_access_class(offset);
        if !matches!(class, crate::MmioAccessClass::FreerunningTimerPoll) {
            let (i, o, n, _) = self.poll_streak.get();
            let (idx, off) = (peri_idx as u32, offset as u32);
            let n = if i == idx && o == off { n } else { 0 };
            self.poll_streak.set((
                idx,
                off,
                n.saturating_add(u64::from(count)),
                self.current_cycle,
            ));
        }
        match class {
            crate::MmioAccessClass::FreerunningTimerPoll => {
                self.freerunning_timer_poll_mmio
                    .set(self.freerunning_timer_poll_mmio.get().saturating_add(count));
            }
            crate::MmioAccessClass::SideEffecting => {
                self.side_effecting_mmio
                    .set(self.side_effecting_mmio.get().saturating_add(count));
            }
            crate::MmioAccessClass::SideEffectFree => {}
        }
    }
}
