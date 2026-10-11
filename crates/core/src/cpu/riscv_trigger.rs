// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ESP32-C3 debug trigger module (hardware breakpoints / watchpoints).
//!
//! The subset of the RISC-V External Debug Support spec (0.13) trigger module
//! the ESP32-C3 implements, per the ESP32-C3 TRM "Debug" / "Trigger Module"
//! sections and ESP-IDF's `soc_caps.h` (`SOC_CPU_BREAKPOINTS_NUM 8`,
//! `SOC_CPU_WATCHPOINTS_NUM 8`, `SOC_CPU_WATCHPOINT_MAX_REGION_SIZE
//! 0x8000_0000`):
//!
//! * [`TRIGGER_COUNT`] triggers, selected by `tselect` (0x7A0, WARL: an
//!   out-of-range index is ignored, so firmware that probes the count by
//!   writing and reading back sees the last legal value).
//! * Every trigger is `mcontrol` (type 2). `tdata1` (0x7A1) holds its config,
//!   `tdata2` (0x7A2) the match address; `tinfo` (0x7A4) reports "type 2 only".
//!   `tdata3` (0x7A3) is not implemented and reads zero.
//! * Match modes: 0 = exact (one of the accessed bytes is at `tdata2`) and
//!   1 = NAPOT (one of the accessed bytes lies in the naturally aligned
//!   power-of-two region `tdata2` encodes). A larger value clips to 1.
//! * Action 0 only (breakpoint exception, `mcause = 3`). Action 1 (enter debug
//!   mode) needs `dmode = 1`, which only Debug Mode can set; there is no debug
//!   module here, so an M-mode write of any non-zero action reads back 0.
//! * `timing = 0`: the trigger fires *before* the matching instruction
//!   executes, so a watched store never reaches memory and `mepc` names the
//!   storing instruction. `mtval` holds the matched address (the data address
//!   for loads/stores, the PC for execute) — esp-hal's exception handler
//!   compares `mtval` with `&__stack_chk_guard` to name a stack-guard hit.
//! * `tcontrol` (0x7A5): `mte` (bit 3) gates every M-mode trigger. Trap entry
//!   saves `mte` to `mpte` (bit 7) and clears `mte`, MRET restores it, so a
//!   trigger never fires inside the handler it would re-enter.
//! * `hit` (bit 20) is set by hardware when the trigger fires and cleared only
//!   by software.

/// Number of triggers the ESP32-C3 implements.
pub const TRIGGER_COUNT: usize = 8;

/// `tdata1.type` = 2 (`mcontrol`), the only type the C3 implements.
const TYPE_MCONTROL: u32 = 2 << 28;
/// `tdata1.maskmax` (bits 26:21): log2 of the largest NAPOT region, 2^31.
const MASKMAX: u32 = 31 << 21;
const HIT: u32 = 1 << 20;
const MATCH_SHIFT: u32 = 7;
const BIT_M: u32 = 1 << 6;
const BIT_U: u32 = 1 << 3;
const BIT_EXECUTE: u32 = 1 << 2;
const BIT_STORE: u32 = 1 << 1;
const BIT_LOAD: u32 = 1 << 0;

/// `tcontrol.mte` / `tcontrol.mpte`.
const MTE: u32 = 1 << 3;
const MPTE: u32 = 1 << 7;

/// `tdata1` at reset: an unconfigured `mcontrol` trigger.
pub const TDATA1_RESET: u32 = TYPE_MCONTROL | MASKMAX;

/// `tinfo`: bit N set means trigger type N is supported. Type 2 only.
const TINFO: u32 = 1 << 2;

/// What a data access does, for matching against `load`/`store`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    Load,
    Store,
    /// An AMO: reads and writes the same word, so either bit matches.
    LoadStore,
}

/// A trigger firing: which trigger, and the address `mtval` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerHit {
    pub index: usize,
    pub address: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerModule {
    tselect: u32,
    tdata1: [u32; TRIGGER_COUNT],
    tdata2: [u32; TRIGGER_COUNT],
    tcontrol: u32,
    /// Cached "could any trigger fire right now" (`mte` and at least one
    /// trigger with `m` and an access-type bit). The interpreter's hot path and
    /// its loop-aggregating fast paths only consult this one bool.
    armed: bool,
}

impl Default for TriggerModule {
    fn default() -> Self {
        Self {
            tselect: 0,
            tdata1: [TDATA1_RESET; TRIGGER_COUNT],
            tdata2: [0; TRIGGER_COUNT],
            tcontrol: 0,
            armed: false,
        }
    }
}

impl TriggerModule {
    /// True if some trigger can fire. When false, no access can trap.
    #[inline]
    pub fn armed(&self) -> bool {
        self.armed
    }

    /// Read a trigger CSR; `None` for addresses outside the module.
    pub fn read_csr(&self, csr: u16) -> Option<u32> {
        let sel = self.tselect as usize;
        Some(match csr {
            0x7A0 => self.tselect,
            0x7A1 => self.tdata1[sel],
            0x7A2 => self.tdata2[sel],
            0x7A3 => 0,
            0x7A4 => TINFO,
            0x7A5 => self.tcontrol,
            _ => return None,
        })
    }

    /// Write a trigger CSR, legalising WARL fields; `false` for addresses
    /// outside the module.
    pub fn write_csr(&mut self, csr: u16, val: u32) -> bool {
        let sel = self.tselect as usize;
        match csr {
            0x7A0 => {
                if (val as usize) < TRIGGER_COUNT {
                    self.tselect = val;
                }
            }
            0x7A1 => self.tdata1[sel] = legalize_tdata1(val),
            0x7A2 => self.tdata2[sel] = val,
            // tdata3 is not implemented; tinfo is read-only.
            0x7A3 | 0x7A4 => {}
            0x7A5 => self.tcontrol = val & (MTE | MPTE),
            _ => return false,
        }
        self.recompute_armed();
        true
    }

    /// Trap entry: `mpte <- mte`, `mte <- 0`.
    pub fn on_trap_entry(&mut self) {
        let mte = self.tcontrol & MTE;
        self.tcontrol = if mte != 0 { MPTE } else { 0 };
        self.recompute_armed();
    }

    /// MRET: `mte <- mpte`.
    pub fn on_mret(&mut self) {
        if self.tcontrol & MPTE != 0 {
            self.tcontrol |= MTE;
        } else {
            self.tcontrol &= !MTE;
        }
        self.recompute_armed();
    }

    /// Check a data access of `len` bytes at `addr`. On a match, sets that
    /// trigger's `hit` bit and returns it; the caller raises the exception.
    pub fn check_data(&mut self, addr: u32, len: u32, kind: AccessKind) -> Option<TriggerHit> {
        if !self.armed {
            return None;
        }
        let want = match kind {
            AccessKind::Load => BIT_LOAD,
            AccessKind::Store => BIT_STORE,
            AccessKind::LoadStore => BIT_LOAD | BIT_STORE,
        };
        self.check(addr, len, want)
    }

    /// Check an instruction fetch of `len` bytes at `pc`.
    pub fn check_execute(&mut self, pc: u32, len: u32) -> Option<TriggerHit> {
        if !self.armed {
            return None;
        }
        self.check(pc, len, BIT_EXECUTE)
    }

    fn check(&mut self, addr: u32, len: u32, want: u32) -> Option<TriggerHit> {
        for i in 0..TRIGGER_COUNT {
            let t1 = self.tdata1[i];
            if t1 & BIT_M == 0 || t1 & want == 0 {
                continue;
            }
            if matches(t1, self.tdata2[i], addr, len) {
                self.tdata1[i] |= HIT;
                return Some(TriggerHit {
                    index: i,
                    address: addr,
                });
            }
        }
        None
    }

    fn recompute_armed(&mut self) {
        self.armed = self.tcontrol & MTE != 0
            && self
                .tdata1
                .iter()
                .any(|&t| t & BIT_M != 0 && t & (BIT_LOAD | BIT_STORE | BIT_EXECUTE) != 0);
    }
}

/// Apply the C3's WARL rules to a value written to `tdata1`.
fn legalize_tdata1(val: u32) -> u32 {
    // type, maskmax: read-only. dmode: Debug-Mode-only, reads 0 here.
    // select/timing/chain/s/h: not implemented, read 0. action: only 0 is
    // valid without dmode. match: 0 or 1, larger values clip to 1.
    let mtch = ((val >> MATCH_SHIFT) & 0xF).min(1);
    TYPE_MCONTROL
        | MASKMAX
        | (val & HIT)
        | (mtch << MATCH_SHIFT)
        | (val & (BIT_M | BIT_U | BIT_EXECUTE | BIT_STORE | BIT_LOAD))
}

/// Does an access of `len` bytes at `addr` match trigger (`tdata1`, `tdata2`)?
fn matches(tdata1: u32, tdata2: u32, addr: u32, len: u32) -> bool {
    let lo = u64::from(addr);
    let hi = lo + u64::from(len.max(1));
    if (tdata1 >> MATCH_SHIFT) & 0xF == 0 {
        // Exact: one of the accessed bytes is at tdata2.
        let t = u64::from(tdata2);
        lo <= t && t < hi
    } else {
        // NAPOT: trailing ones k encode a region of 2^(k+1) bytes, capped at
        // maskmax (2^31).
        let k = tdata2.trailing_ones().min(30);
        let size = 1u64 << (k + 1);
        let base = u64::from(tdata2) & !(size - 1);
        lo < base + size && base < hi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(tm: &mut TriggerModule, idx: u32, tdata1: u32, tdata2: u32) {
        assert!(tm.write_csr(0x7A0, idx));
        assert!(tm.write_csr(0x7A5, MTE));
        assert!(tm.write_csr(0x7A1, tdata1));
        assert!(tm.write_csr(0x7A2, tdata2));
    }

    #[test]
    fn reset_values_and_tinfo() {
        let tm = TriggerModule::default();
        assert_eq!(tm.read_csr(0x7A0), Some(0));
        assert_eq!(tm.read_csr(0x7A1), Some(0x2000_0000 | (31 << 21)));
        assert_eq!(tm.read_csr(0x7A2), Some(0));
        assert_eq!(tm.read_csr(0x7A3), Some(0));
        assert_eq!(tm.read_csr(0x7A4), Some(1 << 2));
        assert_eq!(tm.read_csr(0x7A5), Some(0));
        assert_eq!(tm.read_csr(0x7A6), None);
        assert!(!tm.armed());
    }

    #[test]
    fn tselect_is_warl_and_banks_tdata() {
        let mut tm = TriggerModule::default();
        tm.write_csr(0x7A0, 3);
        tm.write_csr(0x7A2, 0x1234);
        tm.write_csr(0x7A0, TRIGGER_COUNT as u32);
        assert_eq!(tm.read_csr(0x7A0), Some(3), "out-of-range select ignored");
        assert_eq!(tm.read_csr(0x7A2), Some(0x1234));
        tm.write_csr(0x7A0, 0);
        assert_eq!(tm.read_csr(0x7A2), Some(0));
        tm.write_csr(0x7A0, TRIGGER_COUNT as u32 - 1);
        assert_eq!(tm.read_csr(0x7A0), Some(TRIGGER_COUNT as u32 - 1));
    }

    #[test]
    fn tdata1_legalizes_warl_fields() {
        let mut tm = TriggerModule::default();
        // type=0, dmode=1, maskmax=0, action=1, chain=1, match=0xF, s, h, all access bits.
        tm.write_csr(
            0x7A1,
            0x0800_0000 | (1 << 12) | (1 << 11) | (0xF << 7) | 0x7F,
        );
        let v = tm.read_csr(0x7A1).unwrap();
        assert_eq!(v >> 28, 2, "type stays mcontrol");
        assert_eq!(v & (1 << 27), 0, "dmode not writable from M-mode");
        assert_eq!((v >> 21) & 0x3F, 31, "maskmax read-only");
        assert_eq!((v >> 12) & 0xF, 0, "action 1 invalid without dmode");
        assert_eq!(v & (1 << 11), 0, "no chaining");
        assert_eq!((v >> 7) & 0xF, 1, "match clips to NAPOT");
        assert_eq!(v & 0x7F, BIT_M | BIT_U | 0b111, "s/h not implemented");
        // Writing 0 clears the trigger (esp-hal's clear_watchpoint).
        tm.write_csr(0x7A1, 0);
        assert_eq!(tm.read_csr(0x7A1), Some(TDATA1_RESET));
        // hit is software-writable.
        tm.write_csr(0x7A1, HIT);
        assert_eq!(tm.read_csr(0x7A1).unwrap() & HIT, HIT);
    }

    #[test]
    fn tcontrol_trap_and_mret() {
        let mut tm = TriggerModule::default();
        tm.write_csr(0x7A5, 0xFFFF_FFFF);
        assert_eq!(tm.read_csr(0x7A5), Some(MTE | MPTE));
        tm.write_csr(0x7A5, MTE);
        tm.on_trap_entry();
        assert_eq!(tm.read_csr(0x7A5), Some(MPTE));
        tm.on_mret();
        assert_eq!(tm.read_csr(0x7A5), Some(MTE | MPTE));
        tm.write_csr(0x7A5, 0);
        tm.on_trap_entry();
        tm.on_mret();
        assert_eq!(tm.read_csr(0x7A5), Some(0));
    }

    #[test]
    fn napot_store_watch_matches_like_esp_hal() {
        // esp-hal set_watchpoint(0, addr, 4): NAPOT, m, store; tdata2 = addr | 0b01.
        let mut tm = TriggerModule::default();
        let guard = 0x3FC8_1000;
        arm(&mut tm, 0, BIT_M | BIT_STORE | (1 << 7), guard | 0b01);
        assert!(tm.armed());
        assert_eq!(tm.check_data(guard, 4, AccessKind::Load), None);
        assert_eq!(tm.check_data(guard - 4, 4, AccessKind::Store), None);
        assert_eq!(tm.check_data(guard + 4, 4, AccessKind::Store), None);
        let hit = tm.check_data(guard + 3, 1, AccessKind::Store).unwrap();
        assert_eq!(
            hit,
            TriggerHit {
                index: 0,
                address: guard + 3
            }
        );
        assert_eq!(tm.read_csr(0x7A1).unwrap() & HIT, HIT);
        // An unaligned word straddling into the region also matches.
        assert!(tm.check_data(guard - 2, 4, AccessKind::Store).is_some());
        // AMOs match a store watch.
        assert!(tm.check_data(guard, 4, AccessKind::LoadStore).is_some());
    }

    #[test]
    fn exact_match_and_execute() {
        let mut tm = TriggerModule::default();
        arm(&mut tm, 2, BIT_M | BIT_LOAD | BIT_EXECUTE, 0x4200_0102);
        assert!(tm.check_data(0x4200_0100, 2, AccessKind::Load).is_none());
        assert!(tm.check_data(0x4200_0100, 4, AccessKind::Load).is_some());
        assert!(tm.check_data(0x4200_0100, 4, AccessKind::Store).is_none());
        assert_eq!(
            tm.check_execute(0x4200_0102, 2),
            Some(TriggerHit {
                index: 2,
                address: 0x4200_0102
            })
        );
        assert!(tm.check_execute(0x4200_0104, 4).is_none());
    }

    #[test]
    fn disabled_triggers_never_fire() {
        let mut tm = TriggerModule::default();
        // mte = 0: nothing fires even with a fully configured trigger.
        tm.write_csr(0x7A1, BIT_M | BIT_STORE);
        tm.write_csr(0x7A2, 0x100);
        assert!(!tm.armed());
        assert!(tm.check_data(0x100, 4, AccessKind::Store).is_none());
        // m = 0: the trigger does not apply to M-mode.
        tm.write_csr(0x7A5, MTE);
        tm.write_csr(0x7A1, BIT_U | BIT_STORE);
        assert!(!tm.armed());
        assert!(tm.check_data(0x100, 4, AccessKind::Store).is_none());
        // Re-enabled, it fires; after trap entry (mte cleared) it does not.
        tm.write_csr(0x7A1, BIT_M | BIT_STORE);
        assert!(tm.check_data(0x100, 4, AccessKind::Store).is_some());
        tm.on_trap_entry();
        assert!(tm.check_data(0x100, 4, AccessKind::Store).is_none());
    }

    #[test]
    fn napot_large_region() {
        let mut tm = TriggerModule::default();
        // 64 KiB region at 0x4038_0000: 15 trailing ones.
        arm(
            &mut tm,
            1,
            BIT_M | BIT_STORE | (1 << 7),
            0x4038_0000 | 0x7FFF,
        );
        assert!(tm.check_data(0x4037_FFFC, 4, AccessKind::Store).is_none());
        assert!(tm.check_data(0x4038_0000, 1, AccessKind::Store).is_some());
        assert!(tm.check_data(0x4038_FFFF, 1, AccessKind::Store).is_some());
        assert!(tm.check_data(0x4039_0000, 4, AccessKind::Store).is_none());
    }
}
