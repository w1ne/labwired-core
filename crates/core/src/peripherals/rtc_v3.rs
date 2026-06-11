// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! RTC v3 — STM32H5-generation calendar RTC (RM0481 §45).
//!
//! Register map per CMSIS stm32h563xx.h: TR/DR/SSR/ICSR/PRER/WUTR/CR/
//! PRIVCFGR/SECCFGR/WPR/CALR/SHIFTR/TSTR/TSDR/TSSSR/ALRMAR/ALRMASSR/
//! ALRMBR/ALRMBSSR/SR/MISR/SMISR/SCR/OR/ALRABINR/ALRBBINR. This is the
//! "v3" calendar block — split SR/MISR/SCR status banks instead of the
//! single L4 ISR (see [`super::rtc`]), plus binary-mode alarm registers.
//!
//! Reset values pinned per RM0481 and silicon capture 2026-06-11
//! (NUCLEO-H563ZI, fresh backup domain): TR=0, DR=0x2101, SSR=0,
//! ICSR=0x07 (ALRAWF|ALRBWF|WUTWF), PRER=0x007F00FF, WUTR=0xFFFF, CR=0,
//! CALR=0, SR=0, MISR=0, ALRMAR=0; WPR/SHIFTR/SCR are write-only and
//! read 0.
//!
//! Modeling notes (deviations are deliberate and documented inline):
//! - One BCD calendar second elapses every [`TICKS_PER_SECOND`] bus
//!   ticks — a fixed, deterministic divider; PRER does not rescale the
//!   simulated tick rate.
//! - SSR reads as a constant 0. Silicon returned a moving down-counter
//!   (0xF7 captured while running); firmware that only requires SSR
//!   reads not to fault is served, sub-second resolution is not.
//! - Hours wrap 23 -> 00 without a date increment; multi-day runs keep
//!   DR constant (documented simplification).
//! - Timestamp (TSTR/TSDR/TSSSR), wakeup timer expiry, and the
//!   SHIFTR fine-adjust are not modeled; registers are storage/stub.

use crate::{PeripheralTickResult, SimResult};

/// Bus ticks per BCD calendar second (matches a 32.768 kHz LSE fed
/// one-tick-per-cycle; deterministic for tests and trace replay).
pub const TICKS_PER_SECOND: u32 = 32_768;

// ICSR bits (RM0481 §45.6.4).
const ICSR_BASE: u32 = 0x07; // ALRAWF|ALRBWF|WUTWF always read 1
const ICSR_INITS: u32 = 1 << 4;
const ICSR_RSF: u32 = 1 << 5;
const ICSR_INITF: u32 = 1 << 6;
const ICSR_INIT: u32 = 1 << 7;

// CR bits (RM0481 §45.6.7).
const CR_BYPSHAD: u32 = 1 << 5;
const CR_ALRAE: u32 = 1 << 8;
const CR_ALRBE: u32 = 1 << 9;
const CR_ALRAIE: u32 = 1 << 12;
const CR_ALRBIE: u32 = 1 << 13;
const CR_WUTIE: u32 = 1 << 14;
const CR_TSIE: u32 = 1 << 15;

// SR/MISR/SCR bits (RM0481 §45.6.20-45.6.23).
const SR_ALRAF: u32 = 1 << 0;
const SR_ALRBF: u32 = 1 << 1;
const SR_WUTF: u32 = 1 << 2;
const SR_TSF: u32 = 1 << 3;
const SR_TSOVF: u32 = 1 << 4;
const SR_MASK: u32 = 0x7F;

// ALRMxR mask bits.
const ALRM_MSK1: u32 = 1 << 7; // seconds don't-care
const ALRM_MSK2: u32 = 1 << 15; // minutes don't-care
const ALRM_MSK3: u32 = 1 << 23; // hours don't-care
const ALRM_MSK4: u32 = 1 << 31; // date/weekday don't-care
const ALRM_WDSEL: u32 = 1 << 30; // compare weekday instead of date

const TR_MASK: u32 = 0x007F_7F7F;
const DR_MASK: u32 = 0x00FF_FF3F;
const PRER_MASK: u32 = 0x007F_7FFF;
// CALM[8:0] | CALW16(13) | CALW8(14) | CALP(15); LPCAL not modeled.
const CALR_MASK: u32 = 0x0000_E1FF;

/// WPR unlock state machine (RM0481 §45.3.5: write 0xCA then 0x53;
/// any other value re-activates write protection).
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
enum WprState {
    Locked,
    FirstKey,
    Unlocked,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RtcV3 {
    /// Live calendar time, BCD (PM<<22 | HT/HU<<16 | MNT/MNU<<8 | ST/SU).
    tr: u32,
    /// Live calendar date, BCD (YT/YU<<16 | WDU<<13 | MT/MU<<8 | DT/DU).
    dr: u32,
    /// Shadow copies returned when CR.BYPSHAD=0. KISS: re-synced on init
    /// exit and on every calendar second (fine-grained shadow latency —
    /// the up-to-2-RTCCLK staleness window — is not modeled).
    shadow_tr: u32,
    shadow_dr: u32,
    init: bool,
    rsf: bool,
    prer: u32,
    wutr: u32,
    cr: u32,
    privcfgr: u32,
    seccfgr: u32,
    calr: u32,
    alrmar: u32,
    alrmassr: u32,
    alrmbr: u32,
    alrmbssr: u32,
    sr: u32,
    or_reg: u32,
    alrabinr: u32,
    alrbbinr: u32,
    wpr: WprState,
    tick_accum: u32,
}

impl RtcV3 {
    pub fn new() -> Self {
        Self {
            tr: 0,
            dr: 0x0000_2101, // year=00, weekday=Mon, month=01, day=01
            shadow_tr: 0,
            shadow_dr: 0x0000_2101,
            init: false,
            rsf: false,
            prer: 0x007F_00FF,
            wutr: 0x0000_FFFF,
            cr: 0,
            privcfgr: 0,
            seccfgr: 0,
            calr: 0,
            alrmar: 0,
            alrmassr: 0,
            alrmbr: 0,
            alrmbssr: 0,
            sr: 0,
            or_reg: 0,
            alrabinr: 0,
            alrbbinr: 0,
            wpr: WprState::Locked,
            tick_accum: 0,
        }
    }

    fn bypshad(&self) -> bool {
        self.cr & CR_BYPSHAD != 0
    }

    /// ICSR read view. Silicon capture 2026-06-11 (NUCLEO-H563ZI):
    /// 0x07 at reset, 0xC7 in init mode, 0x17 after init exit with a
    /// nonzero year (BYPSHAD=1), 0x27 after init exit with year 00 and
    /// BYPSHAD=0. INITF follows INIT immediately in the model (silicon
    /// rise time is < 1 RTCCLK and not observable at bus granularity).
    fn icsr(&self) -> u32 {
        let mut v = ICSR_BASE;
        if (self.dr >> 16) & 0xFF != 0 {
            v |= ICSR_INITS; // year field nonzero => calendar initialized
        }
        if self.rsf {
            v |= ICSR_RSF;
        }
        if self.init {
            v |= ICSR_INIT | ICSR_INITF;
        }
        v
    }

    /// MISR = SR flags gated by the matching CR interrupt enables.
    fn misr(&self) -> u32 {
        let mut m = 0;
        if self.cr & CR_ALRAIE != 0 {
            m |= self.sr & SR_ALRAF;
        }
        if self.cr & CR_ALRBIE != 0 {
            m |= self.sr & SR_ALRBF;
        }
        if self.cr & CR_WUTIE != 0 {
            m |= self.sr & SR_WUTF;
        }
        if self.cr & CR_TSIE != 0 {
            m |= self.sr & (SR_TSF | SR_TSOVF);
        }
        m
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => {
                if self.bypshad() {
                    self.tr
                } else {
                    self.shadow_tr
                }
            }
            0x04 => {
                if self.bypshad() {
                    self.dr
                } else {
                    self.shadow_dr
                }
            }
            // SSR: constant-0 approximation; silicon read 0xF7 while
            // running. Exact value deliberately unpinned (see module doc).
            0x08 => 0,
            0x0C => self.icsr(),
            0x10 => self.prer,
            0x14 => self.wutr,
            0x18 => self.cr,
            0x1C => self.privcfgr,
            0x20 => self.seccfgr,
            0x24 => 0, // WPR write-only, reads 0 (silicon-pinned)
            0x28 => self.calr,
            0x2C => 0,               // SHIFTR write-only
            0x30 | 0x34 | 0x38 => 0, // TSTR/TSDR/TSSSR: timestamp not modeled
            0x40 => self.alrmar,
            0x44 => self.alrmassr,
            0x48 => self.alrmbr,
            0x4C => self.alrmbssr,
            0x50 => self.sr,
            0x54 => self.misr(),
            0x58 => 0, // SMISR: TrustZone secure view not modeled
            0x5C => 0, // SCR write-only
            0x60 => self.or_reg,
            0x70 => self.alrabinr,
            0x74 => self.alrbbinr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        // WPR key sequence and SCR / ICSR-flag-clears are never gated
        // (RM0481: write protection covers calendar/config registers;
        // SCR and status-flag clearing stay accessible).
        match offset {
            0x24 => {
                // Silicon-pinned: 0xCA then 0x53 unlocks; any other
                // write (including a stray key) re-locks.
                self.wpr = match value & 0xFF {
                    0xCA => WprState::FirstKey,
                    0x53 if self.wpr == WprState::FirstKey => WprState::Unlocked,
                    _ => WprState::Locked,
                };
                return;
            }
            0x0C => {
                // RSF is rc_w0: writing 0 clears, writing 1 leaves it.
                if value & ICSR_RSF == 0 {
                    self.rsf = false;
                }
                // INIT is write-protected (silicon-pinned: INIT write
                // while locked leaves ICSR at its base value).
                if self.wpr == WprState::Unlocked {
                    let want_init = value & ICSR_INIT != 0;
                    if want_init && !self.init {
                        self.init = true; // calendar freezes, INITF rises
                        self.tick_accum = 0;
                    } else if !want_init && self.init {
                        self.init = false;
                        // Calendar restarts from the programmed value.
                        // BYPSHAD=0 (silicon-pinned): shadow syncs and
                        // RSF rises on exit; BYPSHAD=1 leaves RSF clear.
                        if !self.bypshad() {
                            self.sync_shadow();
                        }
                    }
                }
                return;
            }
            0x5C => {
                // SCR: w1c into SR, not write-protected.
                self.sr &= !(value & SR_MASK);
                return;
            }
            // PRIVCFGR/SECCFGR/OR sit outside the WPR domain (privilege /
            // TrustZone gated on silicon; plain storage here).
            0x1C => {
                self.privcfgr = value;
                return;
            }
            0x20 => {
                self.seccfgr = value;
                return;
            }
            0x60 => {
                self.or_reg = value;
                return;
            }
            _ => {}
        }

        if self.wpr != WprState::Unlocked {
            return; // silicon-pinned: locked writes are ignored
        }

        match offset {
            // Calendar registers additionally require init mode
            // (RM0481 §45.3.7; pinned: TR/DR/PRER writable with INITF set).
            0x00 if self.init => {
                self.tr = value & TR_MASK;
                self.shadow_tr = self.tr;
            }
            0x04 if self.init => {
                self.dr = value & DR_MASK;
                self.shadow_dr = self.dr;
            }
            0x10 if self.init => self.prer = value & PRER_MASK,
            0x14 => self.wutr = value,
            0x18 => self.cr = value,
            0x28 => self.calr = value & CALR_MASK,
            // SHIFTR: accepted (write-only) but the sub-second shift has
            // no observable effect — SSR is a constant in this model.
            0x2C => {}
            0x40 => self.alrmar = value,
            0x44 => self.alrmassr = value,
            0x48 => self.alrmbr = value,
            0x4C => self.alrmbssr = value,
            0x70 => self.alrabinr = value,
            0x74 => self.alrbbinr = value,
            _ => {}
        }
    }

    fn sync_shadow(&mut self) {
        self.shadow_tr = self.tr;
        self.shadow_dr = self.dr;
        self.rsf = true;
    }

    /// Advance the live calendar by one second, BCD with carries:
    /// SU 9 -> ST+1; SS 59 -> MM+1; MM 59 -> HH+1; HH wraps 23 -> 00
    /// without a date increment (documented simplification).
    fn increment_second(&mut self) {
        let mut ss = bcd_to_bin(self.tr & 0x7F);
        let mut mm = bcd_to_bin((self.tr >> 8) & 0x7F);
        let mut hh = bcd_to_bin((self.tr >> 16) & 0x3F);
        let pm = self.tr & (1 << 22);
        ss += 1;
        if ss == 60 {
            ss = 0;
            mm += 1;
        }
        if mm == 60 {
            mm = 0;
            hh += 1;
        }
        if hh == 24 {
            hh = 0;
        }
        self.tr = pm | (bin_to_bcd(hh) << 16) | (bin_to_bcd(mm) << 8) | bin_to_bcd(ss);
    }

    /// ALRMxR comparator: each MSKn bit makes its field don't-care.
    /// Sub-second (ALRMxSSR) matching is not modeled — alarms fire on
    /// whole-second boundaries only.
    fn alarm_matches(&self, alrm: u32) -> bool {
        if alrm & ALRM_MSK1 == 0 && (alrm & 0x7F) != (self.tr & 0x7F) {
            return false;
        }
        if alrm & ALRM_MSK2 == 0 && ((alrm >> 8) & 0x7F) != ((self.tr >> 8) & 0x7F) {
            return false;
        }
        if alrm & ALRM_MSK3 == 0 && ((alrm >> 16) & 0x7F) != ((self.tr >> 16) & 0x7F) {
            return false;
        }
        if alrm & ALRM_MSK4 == 0 {
            if alrm & ALRM_WDSEL != 0 {
                if ((alrm >> 24) & 0x7) != ((self.dr >> 13) & 0x7) {
                    return false;
                }
            } else if ((alrm >> 24) & 0x3F) != (self.dr & 0x3F) {
                return false;
            }
        }
        true
    }
}

#[inline]
fn bcd_to_bin(b: u32) -> u32 {
    (b >> 4) * 10 + (b & 0xF)
}

#[inline]
fn bin_to_bcd(b: u32) -> u32 {
    ((b / 10) << 4) | (b % 10)
}

impl Default for RtcV3 {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for RtcV3 {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let sh = ((offset & 3) * 8) as u32;
        Ok(((self.read_reg(reg) >> sh) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let sh = ((offset & 3) * 8) as u32;
        let cur = self.read_reg(reg);
        let merged = (cur & !(0xFFu32 << sh)) | ((value as u32) << sh);
        self.write_reg(reg, merged);
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }
    /// One bus tick. The calendar advances one BCD second every
    /// [`TICKS_PER_SECOND`] ticks while out of init mode (silicon-pinned
    /// running behavior: TR 0x00123456 -> 0x00123457 across a second,
    /// capture 2026-06-11 NUCLEO-H563ZI).
    fn tick(&mut self) -> PeripheralTickResult {
        if self.init {
            return PeripheralTickResult::default();
        }
        self.tick_accum += 1;
        if self.tick_accum < TICKS_PER_SECOND {
            return PeripheralTickResult::default();
        }
        self.tick_accum = 0;
        self.increment_second();
        if !self.bypshad() {
            self.sync_shadow();
        }

        // Alarm comparators run on each second boundary.
        let mut new_flags = 0;
        if self.cr & CR_ALRAE != 0 && self.sr & SR_ALRAF == 0 && self.alarm_matches(self.alrmar) {
            new_flags |= SR_ALRAF;
        }
        if self.cr & CR_ALRBE != 0 && self.sr & SR_ALRBF == 0 && self.alarm_matches(self.alrmbr) {
            new_flags |= SR_ALRBF;
        }
        self.sr |= new_flags;

        // Raise the IRQ line only for newly-set flags whose interrupt
        // enable is on (MISR view of the new flags).
        let enabled = (if self.cr & CR_ALRAIE != 0 {
            SR_ALRAF
        } else {
            0
        }) | (if self.cr & CR_ALRBIE != 0 {
            SR_ALRBF
        } else {
            0
        });
        PeripheralTickResult::with_irq(new_flags & enabled != 0)
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    // Register offsets (CMSIS stm32h563xx.h).
    const TR: u64 = 0x00;
    const DR: u64 = 0x04;
    const SSR: u64 = 0x08;
    const ICSR: u64 = 0x0C;
    const PRER: u64 = 0x10;
    const WUTR: u64 = 0x14;
    const CR: u64 = 0x18;
    const WPR: u64 = 0x24;
    const CALR: u64 = 0x28;
    const SHIFTR: u64 = 0x2C;
    const TSTR: u64 = 0x30;
    const TSDR: u64 = 0x34;
    const TSSSR: u64 = 0x38;
    const ALRMAR: u64 = 0x40;
    const ALRMBR: u64 = 0x48;
    const SR: u64 = 0x50;
    const MISR: u64 = 0x54;
    const SMISR: u64 = 0x58;
    const SCR: u64 = 0x5C;

    const TICKS: u32 = 32_768; // one BCD second

    fn r32(r: &RtcV3, off: u64) -> u32 {
        r.read_u32(off).unwrap()
    }
    fn w32(r: &mut RtcV3, off: u64, v: u32) {
        r.write_u32(off, v).unwrap();
    }
    fn unlock(r: &mut RtcV3) {
        w32(r, WPR, 0xCA);
        w32(r, WPR, 0x53);
    }
    fn tick_second(r: &mut RtcV3) -> bool {
        let mut irq = false;
        for _ in 0..TICKS {
            irq = r.tick().irq;
        }
        irq
    }
    /// Unlock + init + program TR (and optionally DR), CR, then exit init.
    fn program(r: &mut RtcV3, cr: u32, tr: u32, dr: Option<u32>) {
        unlock(r);
        w32(r, CR, cr);
        w32(r, ICSR, 0x80);
        w32(r, TR, tr);
        if let Some(d) = dr {
            w32(r, DR, d);
        }
        w32(r, ICSR, 0x00);
    }

    /// Silicon capture 2026-06-11 (NUCLEO-H563ZI), fresh backup domain.
    #[test]
    fn reset_values_match_silicon() {
        let r = RtcV3::new();
        assert_eq!(r32(&r, TR), 0, "TR");
        assert_eq!(r32(&r, DR), 0x0000_2101, "DR");
        assert_eq!(r32(&r, SSR), 0, "SSR");
        assert_eq!(r32(&r, ICSR), 0x0000_0007, "ICSR = ALRAWF|ALRBWF|WUTWF");
        assert_eq!(r32(&r, PRER), 0x007F_00FF, "PRER");
        assert_eq!(r32(&r, WUTR), 0x0000_FFFF, "WUTR");
        assert_eq!(r32(&r, CR), 0, "CR");
        assert_eq!(r32(&r, WPR), 0, "WPR reads 0");
        assert_eq!(r32(&r, CALR), 0, "CALR");
        assert_eq!(r32(&r, SR), 0, "SR");
        assert_eq!(r32(&r, MISR), 0, "MISR");
        assert_eq!(r32(&r, ALRMAR), 0, "ALRMAR");
        assert_eq!(r32(&r, SCR), 0, "SCR reads 0");
    }

    /// Silicon-pinned: with WPR locked, a TR write is IGNORED.
    #[test]
    fn wpr_locked_tr_write_ignored() {
        let mut r = RtcV3::new();
        w32(&mut r, TR, 0x0012_3456);
        assert_eq!(r32(&r, TR), 0, "locked TR write must be ignored");

        // Same after the calendar holds a configured value.
        program(&mut r, 0x20, 0x0011_2233, None);
        w32(&mut r, WPR, 0xFF); // re-lock
        w32(&mut r, TR, 0); // and try to clobber — must be ignored
        unlock(&mut r);
        w32(&mut r, ICSR, 0x80);
        assert_eq!(r32(&r, TR), 0x0011_2233, "TR kept its value across lock");
    }

    #[test]
    fn wpr_key_sequence_unlocks_wrong_key_relocks() {
        let mut r = RtcV3::new();
        // Wrong sequence first: 0x53 alone must not unlock.
        w32(&mut r, WPR, 0x53);
        w32(&mut r, CR, 0x20);
        assert_eq!(r32(&r, CR), 0, "CR write while locked is ignored");

        unlock(&mut r);
        w32(&mut r, CR, 0x20);
        assert_eq!(r32(&r, CR), 0x20, "CR writable after 0xCA/0x53");

        // Any other WPR write re-locks.
        w32(&mut r, WPR, 0x42);
        w32(&mut r, CR, 0x00);
        assert_eq!(r32(&r, CR), 0x20, "CR write ignored after re-lock");
    }

    #[test]
    fn icsr_init_write_ignored_while_locked() {
        let mut r = RtcV3::new();
        w32(&mut r, ICSR, 0x80);
        assert_eq!(
            r32(&r, ICSR),
            0x07,
            "INIT is WPR-gated; INITF must not rise"
        );
    }

    /// Silicon-pinned init sequence, probed with CR.BYPSHAD=1 (2026-06-11).
    #[test]
    fn init_mode_sequence_pinned_bypshad1() {
        let mut r = RtcV3::new();
        unlock(&mut r);
        w32(&mut r, CR, 0x20);
        assert_eq!(r32(&r, CR), 0x20, "CR.BYPSHAD");

        w32(&mut r, ICSR, 0x80);
        assert_eq!(r32(&r, ICSR), 0x0000_00C7, "INIT|INITF|0x07");

        w32(&mut r, TR, 0x0012_3456);
        assert_eq!(r32(&r, TR), 0x0012_3456, "TR writable in init mode");
        w32(&mut r, DR, 0x0026_0611);
        assert_eq!(r32(&r, DR), 0x0026_0611, "DR writable in init mode");
        w32(&mut r, PRER, 0x007F_00FF);
        assert_eq!(r32(&r, PRER), 0x007F_00FF, "PRER round-trips");

        w32(&mut r, ICSR, 0x00);
        // INITS set (year != 0), RSF NOT set with BYPSHAD=1.
        assert_eq!(r32(&r, ICSR), 0x0000_0017, "ICSR after init exit");
    }

    /// Silicon-pinned: BYPSHAD=0 exit sets RSF; INITS stays 0 (year 00).
    #[test]
    fn init_exit_bypshad0_sets_rsf() {
        let mut r = RtcV3::new();
        unlock(&mut r);
        w32(&mut r, ICSR, 0x80);
        w32(&mut r, TR, 0x0010_1010);
        w32(&mut r, ICSR, 0x00);
        assert_eq!(r32(&r, ICSR), 0x0000_0027, "RSF|0x07, INITS clear");
    }

    /// Silicon-pinned: TR read 0x00123456 then 0x00123457 ~1.1s later.
    #[test]
    fn calendar_ticks_bcd_one_second() {
        let mut r = RtcV3::new();
        program(&mut r, 0x20, 0x0012_3456, Some(0x0026_0611));
        tick_second(&mut r);
        assert_eq!(r32(&r, TR), 0x0012_3457);
        // Sub-second pacing: no increment before the divider elapses.
        for _ in 0..(TICKS - 1) {
            r.tick();
        }
        assert_eq!(r32(&r, TR), 0x0012_3457, "no early increment");
        r.tick();
        assert_eq!(r32(&r, TR), 0x0012_3458);
    }

    #[test]
    fn calendar_bcd_carries() {
        let cases = [
            (0x0012_3459u32, 0x0012_3500u32), // SU 9 -> ST+1, SS 59 -> MM+1
            (0x0012_5959, 0x0013_0000),       // MM 59 -> HH+1
            (0x0023_5959, 0x0000_0000),       // HH wraps 23 -> 00, no date carry
        ];
        for (start, want) in cases {
            let mut r = RtcV3::new();
            program(&mut r, 0x20, start, None);
            tick_second(&mut r);
            assert_eq!(r32(&r, TR), want, "carry from {start:#010X}");
        }
    }

    #[test]
    fn calendar_frozen_in_init_mode() {
        let mut r = RtcV3::new();
        unlock(&mut r);
        w32(&mut r, CR, 0x20);
        w32(&mut r, ICSR, 0x80);
        w32(&mut r, TR, 0x0012_3456);
        tick_second(&mut r);
        tick_second(&mut r);
        assert_eq!(r32(&r, TR), 0x0012_3456, "calendar frozen while INIT=1");
    }

    #[test]
    fn shadow_bypshad0_rsf_set_and_rc_w0_clear() {
        let mut r = RtcV3::new();
        program(&mut r, 0x00, 0x0010_1010, None); // BYPSHAD=0
        assert_eq!(r32(&r, TR), 0x0010_1010, "shadow synced on init exit");
        assert_eq!(r32(&r, ICSR) & 0x20, 0x20, "RSF set");

        // RSF is rc_w0: writing 1 must not set it, writing 0 clears it.
        w32(&mut r, ICSR, 0x00);
        assert_eq!(r32(&r, ICSR), 0x07, "RSF cleared by writing 0");
        w32(&mut r, ICSR, 0x20);
        assert_eq!(r32(&r, ICSR) & 0x20, 0, "RSF cannot be set by software");

        // Each calendar second re-syncs the shadow and re-raises RSF.
        tick_second(&mut r);
        assert_eq!(r32(&r, TR), 0x0010_1011, "shadow follows the calendar");
        assert_eq!(r32(&r, ICSR) & 0x20, 0x20, "RSF re-set on sync");
    }

    #[test]
    fn read_only_and_write_only_registers() {
        let mut r = RtcV3::new();
        unlock(&mut r);
        // Write-only: WPR, SHIFTR, SCR read 0 even after writes.
        w32(&mut r, SHIFTR, 0x8000_7FFF);
        assert_eq!(r32(&r, SHIFTR), 0, "SHIFTR write-only");
        w32(&mut r, SCR, 0x7F);
        assert_eq!(r32(&r, SCR), 0, "SCR write-only");
        assert_eq!(r32(&r, WPR), 0, "WPR write-only");
        // Read-only: writes ignored, reads don't fault (SSR value not pinned).
        for off in [SSR, TSTR, TSDR, TSSSR, SR, MISR, SMISR] {
            w32(&mut r, off, 0xFFFF_FFFF);
            let _ = r32(&r, off);
        }
        assert_eq!(r32(&r, SR), 0, "SR not writable");
        assert_eq!(r32(&r, TSTR), 0, "TSTR reads 0 (timestamp not modeled)");
    }

    #[test]
    fn alarm_a_sets_sr_misr_and_scr_clears() {
        let mut r = RtcV3::new();
        unlock(&mut r);
        w32(&mut r, ICSR, 0x80);
        w32(&mut r, TR, 0x0012_3456);
        // Match on seconds == 57 only (MSK2|MSK3|MSK4 set).
        w32(&mut r, ALRMAR, 0x8080_8057);
        // BYPSHAD | ALRAE | ALRAIE
        w32(&mut r, CR, 0x20 | (1 << 8) | (1 << 12));
        w32(&mut r, ICSR, 0x00);

        let irq = tick_second(&mut r);
        assert_eq!(r32(&r, TR), 0x0012_3457);
        assert_eq!(r32(&r, SR) & 1, 1, "SR.ALRAF set on match");
        assert_eq!(r32(&r, MISR) & 1, 1, "MISR.ALRAMF set (ALRAIE)");
        assert!(irq, "tick raises irq on enabled alarm");

        // SCR is w1c into SR.
        w32(&mut r, SCR, 0x1);
        assert_eq!(r32(&r, SR), 0, "SCR cleared ALRAF");
        assert_eq!(r32(&r, MISR), 0);
    }

    #[test]
    fn byte_access_preserves_unwritten_fields() {
        let mut r = RtcV3::new();
        // WPR unlock via single byte writes must work.
        r.write(WPR, 0xCA).unwrap();
        r.write(WPR, 0x53).unwrap();
        w32(&mut r, ICSR, 0x80);
        w32(&mut r, TR, 0x0010_1010);
        w32(&mut r, ICSR, 0x00); // BYPSHAD=0 -> RSF set
        assert_eq!(r32(&r, ICSR) & 0x20, 0x20);

        // A byte write to ICSR byte 1 must not clobber RSF (byte 0).
        r.write(ICSR + 1, 0x00).unwrap();
        assert_eq!(r32(&r, ICSR) & 0x20, 0x20, "RSF survives high-byte write");
    }

    #[test]
    fn word_access_is_atomic() {
        let mut r = RtcV3::new();
        unlock(&mut r);
        w32(&mut r, ICSR, 0x80);
        w32(&mut r, TR, 0x0012_3456);
        // Both alarms fully masked (MSK1..4): match every second.
        w32(&mut r, ALRMAR, 0x8080_8080);
        w32(&mut r, ALRMBR, 0x8080_8080);
        w32(&mut r, CR, 0x20 | (1 << 8) | (1 << 9));
        w32(&mut r, ICSR, 0x00);
        tick_second(&mut r);
        assert_eq!(r32(&r, SR) & 0x3, 0x3, "ALRAF|ALRBF");

        // One atomic word write to SCR clears both flags at once.
        w32(&mut r, SCR, 0x3);
        assert_eq!(r32(&r, SR), 0);

        // read_u32 view equals the assembled byte view.
        let mut assembled = 0u32;
        for b in 0..4u64 {
            assembled |= (r.read(ICSR + b).unwrap() as u32) << (b * 8);
        }
        assert_eq!(r32(&r, ICSR), assembled);
    }
}
