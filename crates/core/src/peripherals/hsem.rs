// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::collections::HashMap;

/// STM32 Hardware Semaphore (HSEM, RM0434 §31) — the inter-core lock the WB/WL
/// dual-core parts use to arbitrate shared peripherals between CPU1 (Cortex-M4)
/// and CPU2 (Cortex-M0+).
///
/// LabWired runs a single core (CPU1), so every lock is uncontended: a read of a
/// semaphore register grants the lock to CPU1. The 2-step read-lock path Zephyr
/// uses (`z_stm32_hsem_lock` reads `HSEM_RLR[id]` and expects `LOCK | COREID`)
/// therefore succeeds on the first attempt.
///
/// Register layout (offsets from the HSEM base):
/// - `R[0..31]`   at `0x00 + 4*id` — 1-step lock (write COREID|LOCK, read back)
/// - `RLR[0..31]` at `0x80 + 4*id` — 2-step read-lock
/// - `IER/ICR/ISR/MISR/CR/KEYR` at `0x100+` — interrupt/clear/config
///
/// CPU1's HSEM core id is 4; a granted lock reads back as `LOCK(31) | (4<<8)`.
#[derive(Debug, serde::Serialize)]
pub struct Hsem {
    /// Non-lock registers (interrupt/config) are plain read-back storage.
    regs: HashMap<u64, u32>,
}

const LOCK: u32 = 1 << 31;
const CPU1_COREID: u32 = 4;
const GRANTED: u32 = LOCK | (CPU1_COREID << 8);

impl Hsem {
    pub fn new() -> Self {
        Self {
            regs: HashMap::new(),
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        // R[id] (0x00..0x80) and RLR[id] (0x80..0x100): reading grants/reports
        // the lock held by CPU1. Releasing is a write of 0 (stored, not polled),
        // but while a firmware holds the semaphore it expects the granted value.
        if offset < 0x100 {
            return GRANTED;
        }
        self.regs.get(&offset).copied().unwrap_or(0)
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        // Lock/release writes carry no state we need to track in a single-core
        // model; keep the interrupt/config registers as read-back storage.
        if offset >= 0x100 {
            self.regs.insert(offset, value);
        }
    }
}

impl Default for Hsem {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Hsem {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Pure register bank / grant-on-read lock model — no time-driven state.
    /// Class A walk-free: default no-op `tick()` is a genuine no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{Hsem, GRANTED};
    use crate::Peripheral;

    #[test]
    fn rlr_read_grants_lock_to_cpu1() {
        let h = Hsem::new();
        // RLR[5] at 0x80 + 5*4 = 0x94 reads back LOCK | COREID(4).
        assert_eq!(h.read_u32(0x94).unwrap(), GRANTED);
        assert_eq!(GRANTED, 0x8000_0400);
        // R[0] (1-step) likewise grants.
        assert_eq!(h.read_u32(0x00).unwrap(), GRANTED);
    }

    #[test]
    fn interrupt_regs_are_readback_storage() {
        let mut h = Hsem::new();
        h.write_u32(0x110, 0xDEAD_BEEF).unwrap(); // ICR/IER region
        assert_eq!(h.read_u32(0x110).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn class_a_walk_free() {
        let mut h = Hsem::new();
        assert!(!h.needs_legacy_walk());
        // Default tick is a genuine no-op on an armed instance.
        let r = h.tick();
        assert!(!r.irq);
        assert_eq!(r.cycles, 0);
        assert!(r.explicit_irqs.is_none());
    }
}
