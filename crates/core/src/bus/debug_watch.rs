// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

/// One software debug watch range (GDB watchpoint). `base`/`len` are a byte
/// range; `on_read`/`on_store` say which access kind reports a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebugWatch {
    pub base: u64,
    pub len: u64,
    pub on_read: bool,
    pub on_store: bool,
}

impl DebugWatch {
    #[inline]
    pub fn covers(&self, addr: u64) -> bool {
        addr >= self.base && addr < self.base.saturating_add(self.len.max(1))
    }
}

pub const MAX_DEBUG_WATCHES: usize = 16;

impl crate::bus::SystemBus {
    /// Arm one watch range; false when empty, wider than 4 KiB, or at the cap.
    pub fn add_debug_watch(&mut self, base: u64, len: u64, on_read: bool, on_store: bool) -> bool {
        if len == 0 || len > 4096 || self.debug_watches.len() >= MAX_DEBUG_WATCHES {
            return false;
        }
        self.debug_watches.push(DebugWatch {
            base,
            len,
            on_read,
            on_store,
        });
        true
    }

    /// Runs `f` with debug-watch latching disabled. The CPU watches are
    /// DWT-style comparators for CPU accesses; host, debugger and DMA
    /// transfers are not CPU accesses and must not trip them. Nesting-safe;
    /// restores the previous value on every path (the closure's `Err` is a
    /// value here, not an early return).
    pub(crate) fn with_debug_watch_bypass<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.debug_watch_host_bypass.replace(true);
        let r = f(self);
        self.debug_watch_host_bypass.set(saved);
        r
    }

    /// Disarm the exact watch `add_debug_watch` armed; false when absent.
    ///
    /// A pending hit only outlives its watch while some remaining range still
    /// covers it: disarming the watch that latched it must not leave a stale
    /// stop for a later run (see [`Self::take_debug_watch_hit`]).
    pub fn remove_debug_watch(
        &mut self,
        base: u64,
        len: u64,
        on_read: bool,
        on_store: bool,
    ) -> bool {
        match self.debug_watches.iter().position(|w| {
            w.base == base && w.len == len && w.on_read == on_read && w.on_store == on_store
        }) {
            Some(idx) => {
                self.debug_watches.remove(idx);
                if let Some(hit) = self.debug_watch_hit.get() {
                    if !self.debug_watches.iter().any(|w| w.covers(hit)) {
                        self.debug_watch_hit.set(None);
                    }
                }
                true
            }
            None => false,
        }
    }

    /// Drop every watch range and any hit that has not been taken yet.
    pub fn clear_debug_watches(&mut self) {
        self.debug_watches.clear();
        self.debug_watch_hit.set(None);
    }

    /// Consume the first hit latched since the last take.
    pub fn take_debug_watch_hit(&mut self) -> Option<u64> {
        self.debug_watch_hit.take()
    }

    #[inline]
    pub(crate) fn note_debug_watch_store(&mut self, addr: u64) {
        if self.debug_watch_hit.get().is_none()
            && self
                .debug_watches
                .iter()
                .any(|w| w.on_store && w.covers(addr))
        {
            self.debug_watch_hit.set(Some(addr));
        }
    }

    #[inline]
    pub(crate) fn note_debug_watch_read(&self, addr: u64) {
        if self.debug_watch_hit.get().is_none()
            && self
                .debug_watches
                .iter()
                .any(|w| w.on_read && w.covers(addr))
        {
            self.debug_watch_hit.set(Some(addr));
        }
    }
}
