// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! RTC (real-time clock) — STM32F1 layout (RM0008 §18).
//!
//! The F1 RTC is structurally different from the L4/F4 calendar RTC in
//! [`super::rtc`]: it is a plain 32-bit up-counter with a 20-bit prescaler,
//! addressed as 16-bit high/low half-registers (CRH/CRL/PRL/DIV/CNT/ALR)
//! rather than the L4 TR/DR/CR/ISR calendar block. Using the L4 model on an
//! F1 part returns nonsense (e.g. reading CRL@0x04 yields the L4 DR reset
//! 0x2101). This model pins the F1 register map and silicon reset values
//! (verified on a bench STM32F103).
//!
//! Reset values (RM0008 §18.4, cross-checked on silicon): CRH=0, CRL=0x0020
//! (RTOFF set — no write in progress), CNT=0, DIV=0. PRL and ALR are
//! write-only and read back 0.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RtcF1 {
    /// Interrupt-enable register (SECIE/ALRIE/OWIE).
    crh: u32,
    /// Control/status (SECF/ALRF/OWF/RSF/CNF/RTOFF). RTOFF stays set whenever
    /// the model is idle, so RTOFF-polling firmware does not hang.
    crl: u32,
    /// 20-bit prescaler reload (write-only; reads 0).
    prl: u32,
    /// Prescaler divider counter (read-only).
    div: u32,
    /// 32-bit free-running counter (CNTH:CNTL).
    cnt: u32,
    /// 32-bit alarm (write-only; reads 0).
    alr: u32,
}

const RTOFF: u32 = 0x0020;

impl RtcF1 {
    pub fn new() -> Self {
        Self {
            crh: 0,
            crl: RTOFF,
            prl: 0,
            div: 0,
            cnt: 0,
            alr: 0xFFFF_FFFF,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.crh & 0x7,
            0x04 => self.crl & 0x3F,
            0x08 => 0,                         // PRLH — write-only
            0x0C => 0,                         // PRLL — write-only
            0x10 => (self.div >> 16) & 0xFFFF, // DIVH
            0x14 => self.div & 0xFFFF,         // DIVL
            0x18 => (self.cnt >> 16) & 0xFFFF, // CNTH
            0x1C => self.cnt & 0xFFFF,         // CNTL
            0x20 => 0,                         // ALRH — write-only
            0x24 => 0,                         // ALRL — write-only
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.crh = value & 0x7,
            0x04 => {
                // SECF/ALRF/OWF (bits 0-2) are rc_w0 status flags, CNF (4) is
                // R/W, RTOFF (5) is read-only and stays asserted while idle.
                self.crl = (value & 0x1F) | RTOFF;
            }
            0x08 => self.prl = (self.prl & 0x0000_FFFF) | ((value & 0xF) << 16), // PRLH (high 4 bits)
            0x0C => self.prl = (self.prl & 0x000F_0000) | (value & 0xFFFF),      // PRLL
            0x18 => self.cnt = (self.cnt & 0x0000_FFFF) | ((value & 0xFFFF) << 16), // CNTH
            0x1C => self.cnt = (self.cnt & 0xFFFF_0000) | (value & 0xFFFF),      // CNTL
            0x20 => self.alr = (self.alr & 0x0000_FFFF) | ((value & 0xFFFF) << 16), // ALRH
            0x24 => self.alr = (self.alr & 0xFFFF_0000) | (value & 0xFFFF),      // ALRL
            _ => {}
        }
    }
}

impl Default for RtcF1 {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for RtcF1 {
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
    fn needs_legacy_walk(&self) -> bool {
        // The STM32F1 RTC is modelled as a plain register bank: CNT/DIV/CRL only
        // change on MMIO access (there is NO `tick()` override, so the per-cycle
        // walk callback is the default no-op — the counter does not free-run in
        // this model). Deleting the walk is therefore byte-identical for every
        // reachable firmware state (the walk did nothing for it), so the RTC must
        // not pin the whole STM32F1 bus onto the per-cycle walk. This was the last
        // walker on every f103 lab bus — with it gone the full f103 board flips
        // walk-deletable (see the walk_free_campaign / perf acceptance). Mirrors
        // the AFIO no-op-bank fix.
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn read32(r: &RtcF1, off: u64) -> u32 {
        let mut v = 0u32;
        for b in 0..4u64 {
            v |= (r.read(off + b).unwrap() as u32) << (b * 8);
        }
        v
    }

    #[test]
    fn f1_reset_values_match_silicon() {
        let r = RtcF1::new();
        assert_eq!(read32(&r, 0x00), 0, "CRH");
        assert_eq!(read32(&r, 0x04), RTOFF, "CRL RTOFF set");
        assert_eq!(read32(&r, 0x18), 0, "CNTH");
        assert_eq!(read32(&r, 0x1C), 0, "CNTL");
        // CRL is NOT the L4 DR (0x2101) — that was the bug this model fixes.
        assert_ne!(read32(&r, 0x04), 0x2101);
    }

    #[test]
    fn tick_is_a_genuine_no_op_so_walk_is_deletable() {
        // The RTC is a pure register bank: drive it through a boot-like write
        // sequence, then tick it many times — the snapshot must be byte-identical
        // before and after, and the tick result must be default(). This is the
        // inertness proof behind `needs_legacy_walk() == false`.
        let mut r = RtcF1::new();
        r.write(0x00, 0x7).unwrap(); // CRH: all IE bits
        r.write(0x0C, 0xFF).unwrap(); // PRLL
        r.write(0x18, 0x12).unwrap(); // CNTH
        r.write(0x1C, 0x34).unwrap(); // CNTL
        assert!(!r.needs_legacy_walk());
        let before = r.snapshot();
        for _ in 0..1000 {
            let res = r.tick();
            assert!(!res.irq && res.cycles == 0, "tick must be inert");
        }
        assert_eq!(before, r.snapshot(), "no tick may mutate RTC state");
    }

    #[test]
    fn counter_halves_round_trip() {
        let mut r = RtcF1::new();
        for b in 0..2u64 {
            r.write(0x18 + b, ((0xBEEFu32 >> (b * 8)) & 0xFF) as u8)
                .unwrap(); // CNTH = 0xBEEF
            r.write(0x1C + b, ((0x1234u32 >> (b * 8)) & 0xFF) as u8)
                .unwrap(); // CNTL = 0x1234
        }
        assert_eq!(read32(&r, 0x18), 0xBEEF);
        assert_eq!(read32(&r, 0x1C), 0x1234);
    }
}
