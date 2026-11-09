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
//!
//! ## Time base
//!
//! The counter starts at the first write firmware makes (the HAL writes the
//! RTC only after it sets RCC_BDCR.RTCEN). Before that it holds its reset
//! value, as an unclocked RTC does.
//!
//! With a bus [`CycleClock`] in scheduler mode, the counter runs from
//! RTCCLK = [`RTCCLK_HZ`] (the 32.768 kHz LSE) divided by PRL+1, derived
//! lazily from the bus cycle count at `clock_hz` (the CPU clock, 72 MHz on
//! a Blue Pill). Nothing ticks per cycle: CNT, DIV and the SECF/ALRF/OWF
//! flags are computed from the elapsed cycles when firmware reads them.
//! RSF, cleared by firmware, sets again one RTCCLK period later — the wait
//! in HAL `HAL_RTC_WaitForSynchro`, which hung forever while RSF never set.
//!
//! Without a clock (the featureless build) the counter holds still and RSF
//! sets at once, so RSF polls still finish.
//!
//! CNT/PRL/ALR writes take effect only in configuration mode (CRL.CNF=1),
//! as on silicon; RTOFF always reads 1 (writes complete at once).
//!
//! NOT modelled: the SECIE/ALRIE/OWIE interrupts (flags set, no IRQ), the
//! RTCSEL clock source (LSI or HSE/128 run at the LSE rate), RCC_BDCR.RTCEN
//! gating (cold silicon reads CRL as 0; this model reads the 0x0020 reset
//! value), backup-domain reset, and any skew when firmware moves the CPU
//! clock away from `clock_hz`.

use crate::{CycleClock, SimResult};

/// RTCCLK rate: the 32.768 kHz LSE.
pub const RTCCLK_HZ: u64 = 32_768;
/// Default CPU clock the bus cycles run at (STM32F103 at 72 MHz).
pub const DEFAULT_CLOCK_HZ: u64 = 72_000_000;

const SECF: u32 = 1 << 0;
const ALRF: u32 = 1 << 1;
const OWF: u32 = 1 << 2;
const RSF: u32 = 1 << 3;
const CNF: u32 = 1 << 4;
const RTOFF: u32 = 1 << 5;

/// Counter state at one instant.
#[derive(Clone, Copy)]
struct Now {
    cnt: u32,
    /// RTCCLK ticks into the current second (0..=PRL).
    phase: u64,
    /// Flags raised between the anchor and this instant.
    flags: u32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RtcF1 {
    clock_hz: u64,
    /// Interrupt-enable register (SECIE/ALRIE/OWIE).
    crh: u32,
    /// Latched SECF/ALRF/OWF plus CNF; RSF is tracked by `rsf_due`.
    crl: u32,
    /// 20-bit prescaler reload (write-only; reads 0).
    prl: u32,
    /// 32-bit alarm (write-only; reads 0).
    alr: u32,
    /// Counter value and prescaler phase at `anchor_cycle`.
    anchor_cnt: u32,
    anchor_phase: u64,
    anchor_cycle: u64,
    /// Cycle from which RSF reads 1 again; `None` while it is already set.
    rsf_due: Option<u64>,
    /// Set by the first write: the RTC is clocked and the time base runs.
    running: bool,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl RtcF1 {
    pub fn new() -> Self {
        Self::with_clock_hz(DEFAULT_CLOCK_HZ)
    }

    pub fn with_clock_hz(clock_hz: u64) -> Self {
        Self {
            clock_hz: clock_hz.max(1),
            crh: 0,
            crl: 0,
            prl: 0,
            alr: 0xFFFF_FFFF,
            anchor_cnt: 0,
            anchor_phase: 0,
            anchor_cycle: 0,
            // RSF reads 0 out of reset until firmware clears it and the
            // registers resynchronise.
            rsf_due: Some(u64::MAX),
            running: false,
            clock: None,
        }
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        if self.scheduler_mode() {
            self.clock.as_ref().map_or(0, CycleClock::now)
        } else {
            0
        }
    }

    /// RTCCLK ticks elapsed at CPU cycle `cycle`.
    fn ticks_at(&self, cycle: u64) -> u64 {
        (u128::from(cycle) * u128::from(RTCCLK_HZ) / u128::from(self.clock_hz)) as u64
    }

    /// CPU cycles in one RTCCLK period (at least 1).
    fn rtcclk_cycles(&self) -> u64 {
        self.clock_hz.div_ceil(RTCCLK_HZ).max(1)
    }

    /// Counter state at the current cycle.
    fn state(&self) -> Now {
        let elapsed = if self.running {
            self.ticks_at(self.now_cycle())
                .saturating_sub(self.ticks_at(self.anchor_cycle))
        } else {
            0
        };
        let period = u64::from(self.prl) + 1;
        let total = self.anchor_phase + elapsed;
        let secs = total / period;
        let cnt = self.anchor_cnt.wrapping_add(secs as u32);
        let mut flags = 0;
        if secs > 0 {
            flags |= SECF;
            if reached(self.anchor_cnt, self.alr, secs) {
                flags |= ALRF;
            }
            if reached(self.anchor_cnt, 0, secs) {
                flags |= OWF;
            }
        }
        Now {
            cnt,
            phase: total % period,
            flags,
        }
    }

    /// Fold the elapsed time into the anchor, latching any raised flags.
    fn rebase(&mut self) {
        let s = self.state();
        self.crl |= s.flags;
        self.anchor_cnt = s.cnt;
        self.anchor_phase = s.phase;
        self.anchor_cycle = self.now_cycle();
    }

    fn rsf(&self) -> bool {
        self.rsf_due
            .is_none_or(|due| self.scheduler_mode() && self.now_cycle() >= due)
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.crh & 0x7,
            0x04 => {
                let s = self.state();
                let rsf = if self.rsf() { RSF } else { 0 };
                ((self.crl | s.flags) & (SECF | ALRF | OWF | CNF)) | rsf | RTOFF
            }
            0x08 => 0,                                 // PRLH — write-only
            0x0C => 0,                                 // PRLL — write-only
            0x10 => ((self.div() >> 16) & 0xF) as u32, // DIVH
            0x14 => (self.div() & 0xFFFF) as u32,      // DIVL
            0x18 => self.state().cnt >> 16,            // CNTH
            0x1C => self.state().cnt & 0xFFFF,         // CNTL
            0x20 => 0,                                 // ALRH — write-only
            0x24 => 0,                                 // ALRL — write-only
            _ => {
                crate::census_reg!("rtc_f1:RtcF1", offset, "read");
                0
            }
        }
    }

    /// Prescaler divider: counts PRL down to 0 once per second.
    fn div(&self) -> u64 {
        u64::from(self.prl) - self.state().phase.min(u64::from(self.prl))
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if !self.running {
            self.running = true;
            self.anchor_cycle = self.now_cycle();
        }
        let cnf = self.crl & CNF != 0;
        match offset {
            0x00 => self.crh = value & 0x7,
            0x04 => {
                self.rebase();
                // SECF/ALRF/OWF/RSF are rc_w0: writing 0 clears, 1 keeps.
                // CNF is R/W; RTOFF is read-only.
                let keep = (value & (SECF | ALRF | OWF)) | !(SECF | ALRF | OWF);
                self.crl = (self.crl & keep & !CNF) | (value & CNF);
                if value & RSF == 0 {
                    // Without a clock the resynchronisation is immediate.
                    self.rsf_due = self
                        .scheduler_mode()
                        .then(|| self.now_cycle() + self.rtcclk_cycles());
                } else if self.rsf() {
                    self.rsf_due = None;
                }
            }
            0x08 if cnf => {
                self.rebase();
                self.prl = (self.prl & 0xFFFF) | ((value & 0xF) << 16); // PRLH
                self.anchor_phase = self.anchor_phase.min(u64::from(self.prl));
            }
            0x0C if cnf => {
                self.rebase();
                self.prl = (self.prl & 0xF_0000) | (value & 0xFFFF); // PRLL
                self.anchor_phase = self.anchor_phase.min(u64::from(self.prl));
            }
            0x18 if cnf => {
                self.rebase();
                self.anchor_cnt = (self.anchor_cnt & 0xFFFF) | ((value & 0xFFFF) << 16);
            }
            0x1C if cnf => {
                self.rebase();
                self.anchor_cnt = (self.anchor_cnt & 0xFFFF_0000) | (value & 0xFFFF);
            }
            0x20 if cnf => {
                self.rebase();
                self.alr = (self.alr & 0xFFFF) | ((value & 0xFFFF) << 16);
            }
            0x24 if cnf => {
                self.rebase();
                self.alr = (self.alr & 0xFFFF_0000) | (value & 0xFFFF);
            }
            // Outside configuration mode these writes are ignored.
            0x08 | 0x0C | 0x18 | 0x1C | 0x20 | 0x24 => {}
            _ => {
                crate::census_reg!("rtc_f1:RtcF1", offset, "write");
            }
        }
    }
}

/// Whether a counter at `from` steps onto `target` within `secs` increments.
fn reached(from: u32, target: u32, secs: u64) -> bool {
    let distance = target.wrapping_sub(from);
    distance != 0 && u64::from(distance) <= secs
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
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }
    fn needs_legacy_walk(&self) -> bool {
        // The counter is derived lazily from the bus cycle clock on every
        // read; there is no `tick()` override and no per-cycle state, so the
        // RTC must not pin the STM32F1 bus onto the per-cycle walk.
        false
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
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

    fn w(r: &mut RtcF1, off: u64, v: u32) {
        r.write_u32(off, v).unwrap();
    }

    fn cnt(r: &RtcF1) -> u32 {
        (read32(r, 0x18) << 16) | read32(r, 0x1C)
    }

    #[test]
    fn f1_reset_values_match_silicon() {
        let r = RtcF1::new();
        assert_eq!(read32(&r, 0x00), 0, "CRH");
        assert_eq!(read32(&r, 0x04), RTOFF, "CRL RTOFF set, RSF clear");
        assert_eq!(read32(&r, 0x18), 0, "CNTH");
        assert_eq!(read32(&r, 0x1C), 0, "CNTL");
        // CRL is NOT the L4 DR (0x2101) — that was the bug this model fixes.
        assert_ne!(read32(&r, 0x04), 0x2101);
    }

    #[test]
    fn tick_is_a_genuine_no_op_so_walk_is_deletable() {
        // The RTC is derived from the cycle clock, never from ticks: drive it
        // through a boot-like write sequence, then tick it many times — the
        // snapshot must be byte-identical before and after.
        let mut r = RtcF1::new();
        w(&mut r, 0x04, CNF);
        w(&mut r, 0x00, 0x7); // CRH: all IE bits
        w(&mut r, 0x0C, 0x7FFF); // PRLL
        w(&mut r, 0x18, 0x12); // CNTH
        w(&mut r, 0x1C, 0x34); // CNTL
        w(&mut r, 0x04, 0);
        assert!(!r.needs_legacy_walk());
        let before = r.snapshot();
        for _ in 0..1000 {
            let res = r.tick();
            assert!(!res.irq && res.cycles == 0, "tick must be inert");
        }
        assert_eq!(before, r.snapshot(), "no tick may mutate RTC state");
    }

    #[test]
    fn counter_halves_round_trip_in_config_mode_only() {
        let mut r = RtcF1::new();
        w(&mut r, 0x18, 0xBEEF);
        assert_eq!(cnt(&r), 0, "CNT write outside CNF is ignored");
        w(&mut r, 0x04, CNF);
        for b in 0..2u64 {
            r.write(0x18 + b, ((0xBEEFu32 >> (b * 8)) & 0xFF) as u8)
                .unwrap(); // CNTH = 0xBEEF
            r.write(0x1C + b, ((0x1234u32 >> (b * 8)) & 0xFF) as u8)
                .unwrap(); // CNTL = 0x1234
        }
        assert_eq!(read32(&r, 0x18), 0xBEEF);
        assert_eq!(read32(&r, 0x1C), 0x1234);
    }

    #[test]
    fn rsf_sets_again_without_a_clock() {
        // HAL_RTC_WaitForSynchro: clear RSF, poll until it is set.
        let mut r = RtcF1::new();
        w(&mut r, 0x04, !RSF & 0xFFFF);
        assert_ne!(read32(&r, 0x04) & RSF, 0);
    }

    #[cfg(feature = "event-scheduler")]
    mod clocked {
        use super::*;

        const HZ: u64 = 72_000_000;

        fn clocked() -> (RtcF1, CycleClock) {
            let clock = CycleClock::default();
            let mut r = RtcF1::with_clock_hz(HZ);
            r.attach_cycle_clock(clock.clone());
            (r, clock)
        }

        /// The HAL F1 init: CNF, PRL = 32767, CNT, ALR, leave CNF.
        fn init(r: &mut RtcF1, cnt: u32, alr: u32) {
            w(r, 0x04, CNF | RSF | SECF | ALRF | OWF);
            w(r, 0x08, 0);
            w(r, 0x0C, 0x7FFF);
            w(r, 0x18, cnt >> 16);
            w(r, 0x1C, cnt & 0xFFFF);
            w(r, 0x20, alr >> 16);
            w(r, 0x24, alr & 0xFFFF);
            w(r, 0x04, RSF | SECF | ALRF | OWF);
        }

        #[test]
        fn counter_runs_one_second_per_32768_rtcclk() {
            let (mut r, clock) = clocked();
            init(&mut r, 45_296, 0xFFFF_FFFF);
            assert_eq!(cnt(&r), 45_296);
            clock.publish(HZ - 1);
            assert_eq!(cnt(&r), 45_296, "not yet one second");
            clock.publish(HZ);
            assert_eq!(cnt(&r), 45_297);
            clock.publish(HZ * 5 / 2);
            assert_eq!(cnt(&r), 45_298);
            // DIV counts PRL down: half a second in, about 0x3FFF remain.
            let div = (read32(&r, 0x10) << 16) | read32(&r, 0x14);
            assert!((0x3FFE..=0x4000).contains(&div), "DIV {div:#x}");
        }

        #[test]
        fn counter_holds_still_until_the_first_write() {
            let (mut r, clock) = clocked();
            clock.publish(10 * HZ);
            assert_eq!(cnt(&r), 0, "not clocked yet: CNT stays at reset");
            init(&mut r, 7, 0xFFFF_FFFF);
            clock.publish(11 * HZ);
            assert_eq!(cnt(&r), 8, "one second after the first write");
        }

        #[test]
        fn rsf_sets_one_rtcclk_after_clear() {
            let (mut r, clock) = clocked();
            clock.publish(1000);
            w(&mut r, 0x04, 0);
            assert_eq!(read32(&r, 0x04) & RSF, 0, "cleared");
            clock.publish(1000 + HZ.div_ceil(RTCCLK_HZ));
            assert_ne!(read32(&r, 0x04) & RSF, 0, "resynchronised");
        }

        #[test]
        fn secf_and_alrf_latch_and_clear_by_writing_zero() {
            let (mut r, clock) = clocked();
            init(&mut r, 10, 12);
            clock.publish(HZ);
            let crl = read32(&r, 0x04);
            assert_ne!(crl & SECF, 0);
            assert_eq!(crl & ALRF, 0, "CNT 11 < ALR 12");
            clock.publish(2 * HZ);
            assert_ne!(read32(&r, 0x04) & ALRF, 0, "CNT reached 12");
            w(&mut r, 0x04, RSF | ALRF); // clear SECF only
            let crl = read32(&r, 0x04);
            assert_eq!(crl & SECF, 0);
            assert_ne!(crl & ALRF, 0, "ALRF latched");
            clock.publish(3 * HZ);
            assert_ne!(read32(&r, 0x04) & SECF, 0, "next second");
            assert_eq!(cnt(&r), 13);
        }

        #[test]
        fn counter_overflow_sets_owf() {
            let (mut r, clock) = clocked();
            init(&mut r, 0xFFFF_FFFF, 5);
            clock.publish(HZ);
            assert_eq!(cnt(&r), 0);
            assert_ne!(read32(&r, 0x04) & OWF, 0);
        }
    }
}
