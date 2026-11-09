// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM4S system-controller blocks every boot path touches: the Watchdog
//! Timer (WDT), the Reset Controller (RSTC) and the Enhanced Embedded Flash
//! Controller (EEFC). Register maps and bit meanings are from the SAM4S
//! datasheet (Atmel-11100, §16 RSTC, §18 WDT, §20 EEFC); Renode's
//! `SAM4S_WDT`/`SAM4S_RSTC`/`SAM4S_EEFC` were read as a second source.
//!
//! WDT: the 12-bit down-counter runs at SLCK/128 (256 Hz) from reset — the
//! silicon default WDT_MR is 0x3FFF_2FFF, so the watchdog is ARMED at
//! power-on with a ~16 s period and WDRSTEN set, unlike Renode, which starts
//! it stopped. WDT_MR is write-once until a processor reset; writing it
//! reloads the counter. WDT_CR (key 0xA5) WDRSTT restarts it; a restart while
//! the counter is above WDD (with WDD < WDV) is a window error (WDERR).
//! Reaching 0 (WDV counts after a load) sets WDUNF and reloads WDV. Either
//! fault asserts the level IRQ when WDFIEN is set and requests a watchdog
//! reset when WDRSTEN is set.
//! WDT_SR read clears both flags. The counter is lazy (computed from the CPU
//! cycle clock, SLCK scaled off `cpu_hz`). Not modelled: WDDBGHLT/WDIDLEHLT
//! (stored; the counter keeps running while the core sleeps) and WDRPROC
//! (every watchdog reset reboots core and peripherals).
//!
//! RSTC: RSTC_CR (key 0xA5) with PROCRST requests a reboot; RSTC_SR.RSTTYP
//! records why the core last restarted (0 general, 2 watchdog, 3 software —
//! RSTC_CR and AIRCR.SYSRESETREQ both report software). RSTTYP survives
//! system resets. NRSTL reads 1 (NRST is not wired); RSTC_MR is stored.
//! PERRST/EXTRST alone (without PROCRST) and the user-reset path are not
//! modelled: the bus reset path cannot reset peripherals without the core.
//!
//! EEFC: EEFC_FMR is stored (FWS, SCOD, FAM, CLOE, FRDY interrupt enable),
//! FSR.FRDY always reads 1 (commands complete at once). EEFC_FCR (key 0x5A)
//! executes GETD (descriptor read back through EEFC_FRR), GLB/SLB/CLB (lock
//! bits, one per 8 KiB region) and GGPB/SGPB/CGPB (two GPNVM bits; GPNVM1
//! reads 1 — this engine boots from flash). A bad key or unknown command sets
//! FCMDE. Program/erase commands (WP, WPL, EWP, EWPL, EA, EPA, ES, the user
//! signature and unique-ID modes, GCALB) are REFUSED with an error: CPU stores
//! into flash commit straight to the array in this engine, so a page-buffer
//! controller cannot be honestly claimed. FL_ID in the descriptor is 0 (the
//! datasheet does not publish it).

use crate::{CycleClock, Peripheral, PeripheralResetCause, PeripheralTickResult, SimResult};
use std::cell::Cell;

const SLCK_HZ: u128 = 32_768;
const WDT_PRESCALE: u128 = 128;
const KEY_A5: u32 = 0xA5;

fn lane(value: u32, offset: u64) -> u8 {
    (value >> ((offset & 3) * 8)) as u8
}

fn merge_lane(value: u32, offset: u64, byte: u8) -> u32 {
    let shift = (offset & 3) * 8;
    (value & !(0xFF << shift)) | (u32::from(byte) << shift)
}

// ── WDT ──────────────────────────────────────────────────────────────────

const WDT_CR: u64 = 0x00;
const WDT_MR: u64 = 0x04;
const WDT_SR: u64 = 0x08;
/// WDT_MR power-on value: WDV = WDD = 0xFFF, WDRSTEN, WDDBGHLT, WDIDLEHLT.
pub const WDT_MR_RESET: u32 = 0x3FFF_2FFF;
const MR_WDFIEN: u32 = 1 << 12;
const MR_WDRSTEN: u32 = 1 << 13;
const MR_WDDIS: u32 = 1 << 15;
const SR_WDUNF: u32 = 1 << 0;
const SR_WDERR: u32 = 1 << 1;

/// SAM4S Watchdog Timer. The state that moves with time sits in `Cell`s so
/// a WDT_SR read (`&self`) can apply an elapsed underflow before it clears.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Sam4sWdt {
    cpu_hz: u64,
    mr: u32,
    mr_locked: bool,
    sr: Cell<u32>,
    /// CPU cycle the counter last reloaded from WDV; `None` while disabled.
    loaded_at: Cell<Option<u64>>,
    reset_latched: Cell<bool>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Sam4sWdt {
    pub fn new() -> Self {
        let wdt = Self {
            cpu_hz: 4_000_000,
            mr: WDT_MR_RESET,
            mr_locked: false,
            sr: Cell::new(0),
            loaded_at: Cell::new(None),
            reset_latched: Cell::new(false),
            clock: None,
        };
        wdt.reload();
        wdt
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn wdv(&self) -> u32 {
        self.mr & 0xFFF
    }

    fn wdd(&self) -> u32 {
        (self.mr >> 16) & 0xFFF
    }

    fn reload(&self) {
        self.loaded_at
            .set((self.mr & MR_WDDIS == 0).then(|| self.now()));
    }

    /// Whole watchdog counts (SLCK/128) between two CPU cycles.
    fn counts_between(&self, from: u64, to: u64) -> u64 {
        (u128::from(to.saturating_sub(from)) * SLCK_HZ
            / (WDT_PRESCALE * u128::from(self.cpu_hz.max(1)))) as u64
    }

    /// CPU cycle at which the counter, loaded at `at`, underflows.
    fn underflow_cycle(&self, at: u64) -> u64 {
        // The fault lands when the counter reaches 0, WDV counts after a load.
        let counts = u128::from(self.wdv().max(1));
        let cycles = (counts * WDT_PRESCALE * u128::from(self.cpu_hz.max(1))).div_ceil(SLCK_HZ);
        at.saturating_add(cycles as u64)
    }

    /// The counter value now (WDV while disabled).
    pub fn counter(&self) -> u32 {
        self.advance();
        match self.loaded_at.get() {
            Some(at) => {
                let elapsed = self.counts_between(at, self.now());
                self.wdv().saturating_sub(elapsed.min(0xFFF) as u32)
            }
            None => self.wdv(),
        }
    }

    fn fault(&self, flag: u32) {
        self.sr.set(self.sr.get() | flag);
        if self.mr & MR_WDRSTEN != 0 {
            self.reset_latched.set(true);
        }
    }

    /// Apply every underflow that has elapsed by now; each reloads WDV.
    fn advance(&self) {
        while let Some(at) = self.loaded_at.get() {
            let due = self.underflow_cycle(at);
            if self.now() < due {
                return;
            }
            self.fault(SR_WDUNF);
            self.loaded_at.set(Some(due));
            if self.reset_latched.get() {
                return;
            }
        }
    }

    fn line(&self) -> bool {
        self.mr & MR_WDFIEN != 0 && self.sr.get() & (SR_WDUNF | SR_WDERR) != 0
    }

    fn read_reg(&self, offset: u64, clear: bool) -> u32 {
        self.advance();
        match offset {
            WDT_MR => self.mr,
            WDT_SR => {
                let sr = self.sr.get();
                if clear {
                    self.sr.set(0);
                }
                sr
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        self.advance();
        match offset {
            WDT_CR if value >> 24 == KEY_A5 && value & 1 != 0 => {
                if self.loaded_at.get().is_none() {
                    return;
                }
                if self.wdd() < self.wdv() && self.counter() > self.wdd() {
                    self.fault(SR_WDERR);
                    return;
                }
                self.reload();
            }
            WDT_MR if !self.mr_locked => {
                self.mr = value & 0x3FFF_FFFF;
                self.mr_locked = true;
                self.reload();
            }
            _ => {}
        }
    }

    fn power_on(&mut self) {
        let (clock, cpu_hz) = (self.clock.take(), self.cpu_hz);
        *self = Self::new();
        self.cpu_hz = cpu_hz;
        self.clock = clock;
        self.reload();
    }
}

impl Default for Sam4sWdt {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Sam4sWdt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = offset & !3;
        Ok(lane(self.read_reg(aligned, offset == aligned), offset))
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3, true))
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        Some(lane(self.read_reg(offset & !3, false), offset))
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let merged = merge_lane(self.read_reg(aligned, false), offset, value);
        self.write_reg(aligned, merged);
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }
    /// The walk only delivers the WDFIEN interrupt; resets drain through
    /// `take_reset_request` at instruction boundaries.
    fn legacy_tick_active(&self) -> bool {
        self.mr & MR_WDFIEN != 0 && (self.loaded_at.get().is_some() || self.line())
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }
    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.advance();
        PeripheralTickResult::with_irq(self.line())
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.line())
    }
    fn reads_can_deassert_irq(&self) -> bool {
        self.mr & MR_WDFIEN != 0
    }
    fn reset_request_capable(&self) -> bool {
        true
    }
    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        self.advance();
        self.reset_latched
            .get()
            .then_some(PeripheralResetCause::Watchdog)
    }
    fn take_reset_request(&mut self) -> bool {
        self.advance();
        self.reset_latched.replace(false)
    }
    fn on_system_reset(&mut self) {
        self.power_on();
    }
    fn on_node_reset(&mut self) {
        self.power_on();
    }
    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
        self.reload();
    }
    /// A chip without a declared `cpu_hz` reports 0; keep the 4 MHz fast-RC
    /// reset clock then rather than scaling SLCK off a 1 Hz core.
    fn attach_cpu_hz(&mut self, hz: u64) {
        if hz != 0 {
            self.cpu_hz = hz;
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "sam4s_wdt", "mr": self.mr,
            "mr_locked": self.mr_locked, "sr": self.sr.get(),
            "counter": self.counter(), "enabled": self.loaded_at.get().is_some()})
    }
}

// ── RSTC ─────────────────────────────────────────────────────────────────

const RSTC_CR: u64 = 0x00;
const RSTC_SR: u64 = 0x04;
const RSTC_MR: u64 = 0x08;
const CR_PROCRST: u32 = 1 << 0;
const SR_NRSTL: u32 = 1 << 16;
pub const RSTTYP_GENERAL: u32 = 0;
pub const RSTTYP_WATCHDOG: u32 = 2;
pub const RSTTYP_SOFTWARE: u32 = 3;

/// SAM4S Reset Controller.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Sam4sRstc {
    rsttyp: u32,
    mr: u32,
    reset_latched: bool,
}

impl Sam4sRstc {
    pub fn new() -> Self {
        Self {
            rsttyp: RSTTYP_GENERAL,
            mr: 1,
            reset_latched: false,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            RSTC_SR => SR_NRSTL | (self.rsttyp << 8),
            RSTC_MR => self.mr & 0x0000_0F11,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if value >> 24 != KEY_A5 {
            return;
        }
        match offset {
            RSTC_CR if value & CR_PROCRST != 0 => self.reset_latched = true,
            RSTC_MR => self.mr = value & 0x0000_0F11,
            _ => {}
        }
    }
}

impl Default for Sam4sRstc {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Sam4sRstc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane(self.read_reg(offset & !3), offset))
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // The KEY sits in the top byte, so only a full word can carry it.
        let _ = (offset, value);
        Ok(())
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }
    fn reset_request_capable(&self) -> bool {
        true
    }
    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        self.reset_latched.then_some(PeripheralResetCause::System)
    }
    fn take_reset_request(&mut self) -> bool {
        std::mem::take(&mut self.reset_latched)
    }
    fn on_peripheral_reset(&mut self, cause: PeripheralResetCause) {
        self.rsttyp = match cause {
            PeripheralResetCause::Watchdog => RSTTYP_WATCHDOG,
            PeripheralResetCause::System => RSTTYP_SOFTWARE,
        };
    }
    fn on_node_reset(&mut self) {
        *self = Self::new();
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "sam4s_rstc", "rsttyp": self.rsttyp, "mr": self.mr})
    }
}

// ── EEFC ─────────────────────────────────────────────────────────────────

const EEFC_FMR: u64 = 0x00;
const EEFC_FCR: u64 = 0x04;
const EEFC_FSR: u64 = 0x08;
const EEFC_FRR: u64 = 0x0C;
const FSR_FRDY: u32 = 1 << 0;
const FSR_FCMDE: u32 = 1 << 1;
const FSR_FLOCKE: u32 = 1 << 2;
const FMR_WRITABLE: u32 = 0x0501_0F01;
const KEY_5A: u32 = 0x5A;
pub const EEFC_PAGE: u32 = 512;
pub const EEFC_LOCK_REGION: u32 = 8 * 1024;

/// SAM4S Enhanced Embedded Flash Controller (one plane).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Sam4sEefc {
    flash_size: u32,
    fmr: u32,
    fsr: Cell<u32>,
    /// GETD/GLB/GGPB results, consumed one word per EEFC_FRR read.
    result: Vec<u32>,
    result_pos: Cell<usize>,
    lock_bits: Vec<bool>,
    gpnvm: u32,
}

impl Sam4sEefc {
    pub fn new(flash_size: u32) -> Self {
        let regions = (flash_size / EEFC_LOCK_REGION).max(1) as usize;
        Self {
            flash_size,
            fmr: 0,
            fsr: Cell::new(0),
            result: Vec::new(),
            result_pos: Cell::new(0),
            lock_bits: vec![false; regions],
            gpnvm: 0b10,
        }
    }

    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(crate::SimulationError::NotImplemented(format!(
            "sam4s_eefc: {what} is not modelled (flash stores commit directly here)"
        )))
    }

    fn set_result(&mut self, words: Vec<u32>) {
        self.result = words;
        self.result_pos.set(0);
    }

    fn page_region(&self, page: u32) -> Option<usize> {
        let index = (page * EEFC_PAGE / EEFC_LOCK_REGION) as usize;
        (page * EEFC_PAGE < self.flash_size).then_some(index)
    }

    fn command(&mut self, value: u32) -> SimResult<()> {
        let (cmd, arg) = (value & 0xFF, (value >> 8) & 0xFFFF);
        if value >> 24 != KEY_5A {
            self.fsr.set(self.fsr.get() | FSR_FCMDE);
            return Ok(());
        }
        match cmd {
            0x00 => {
                let mut d = vec![
                    0,
                    self.flash_size,
                    EEFC_PAGE,
                    1,
                    self.flash_size,
                    self.lock_bits.len() as u32,
                ];
                d.extend(std::iter::repeat_n(EEFC_LOCK_REGION, self.lock_bits.len()));
                self.set_result(d);
            }
            0x08 | 0x09 => match self.page_region(arg) {
                Some(r) => self.lock_bits[r] = cmd == 0x08,
                None => self.fsr.set(self.fsr.get() | FSR_FCMDE),
            },
            0x0A => {
                let word = self
                    .lock_bits
                    .iter()
                    .enumerate()
                    .fold(0u32, |w, (i, &l)| w | (u32::from(l) << (i % 32)));
                self.set_result(vec![word]);
            }
            0x0B | 0x0C if arg < 2 => {
                if cmd == 0x0B {
                    self.gpnvm |= 1 << arg;
                } else {
                    self.gpnvm &= !(1 << arg);
                }
            }
            0x0B | 0x0C => self.fsr.set(self.fsr.get() | FSR_FCMDE),
            0x0D => self.set_result(vec![self.gpnvm]),
            0x01..=0x05 | 0x07 | 0x0E..=0x15 => {
                return Self::refuse(&format!("flash command 0x{cmd:02X}"))
            }
            _ => self.fsr.set(self.fsr.get() | FSR_FCMDE),
        }
        Ok(())
    }

    fn read_reg(&self, offset: u64, consume: bool) -> u32 {
        match offset {
            EEFC_FMR => self.fmr,
            EEFC_FSR => {
                let fsr = self.fsr.get();
                if consume {
                    self.fsr.set(fsr & !(FSR_FCMDE | FSR_FLOCKE));
                }
                fsr | FSR_FRDY
            }
            EEFC_FRR => {
                let pos = self.result_pos.get();
                let word = self.result.get(pos).copied().unwrap_or(0);
                if consume && pos < self.result.len() {
                    self.result_pos.set(pos + 1);
                }
                word
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            EEFC_FMR => self.fmr = value & FMR_WRITABLE,
            EEFC_FCR => self.command(value)?,
            _ => {}
        }
        Ok(())
    }
}

impl Peripheral for Sam4sEefc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = offset & !3;
        Ok(lane(self.read_reg(aligned, offset == aligned), offset))
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3, true))
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        Some(lane(self.read_reg(offset & !3, false), offset))
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        if aligned == EEFC_FCR {
            return Self::refuse("a byte store to EEFC_FCR");
        }
        let merged = merge_lane(self.read_reg(aligned, false), offset, value);
        self.write_reg(aligned, merged)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value)
    }
    fn on_node_reset(&mut self) {
        *self = Self::new(self.flash_size);
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"peripheral": "sam4s_eefc", "fmr": self.fmr,
            "fsr": self.fsr.get(), "gpnvm": self.gpnvm,
            "locked_regions": self.lock_bits.iter().filter(|&&l| l).count()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adv(clock: &CycleClock, cycles: u64) {
        clock.publish(clock.now() + cycles);
    }

    fn wdt_at(hz: u64) -> (Sam4sWdt, CycleClock) {
        let clock = CycleClock::default();
        let mut wdt = Sam4sWdt::new();
        wdt.attach_cpu_hz(hz);
        wdt.attach_cycle_clock(clock.clone());
        (wdt, clock)
    }

    #[test]
    fn wdt_is_armed_from_reset_and_resets_after_wdv_counts() {
        // 32768 Hz CPU: one watchdog count is 128 cycles.
        let (mut wdt, clock) = wdt_at(32_768);
        assert_eq!(wdt.read_u32(WDT_MR).unwrap(), WDT_MR_RESET);
        adv(&clock, 128 * 0xFFF - 1);
        assert!(!wdt.take_reset_request());
        adv(&clock, 1);
        assert_eq!(
            wdt.latched_reset_cause(),
            Some(PeripheralResetCause::Watchdog)
        );
        assert!(wdt.take_reset_request());
        assert_eq!(wdt.read_u32(WDT_SR).unwrap() & SR_WDUNF, SR_WDUNF);
        assert_eq!(wdt.read_u32(WDT_SR).unwrap(), 0, "SR read clears");
    }

    #[test]
    fn wdt_mr_is_write_once_and_wddis_stops_it() {
        let (mut wdt, clock) = wdt_at(32_768);
        wdt.write_u32(WDT_MR, MR_WDDIS).unwrap();
        wdt.write_u32(WDT_MR, 0x0000_2010).unwrap();
        assert_eq!(wdt.read_u32(WDT_MR).unwrap(), MR_WDDIS);
        adv(&clock, 128 * 0x2000);
        assert!(!wdt.take_reset_request());
        wdt.on_system_reset();
        assert_eq!(
            wdt.read_u32(WDT_MR).unwrap(),
            WDT_MR_RESET,
            "processor reset unlocks"
        );
    }

    #[test]
    fn wdt_restart_needs_the_key_and_reloads() {
        let (mut wdt, clock) = wdt_at(32_768);
        wdt.write_u32(WDT_MR, MR_WDRSTEN | (0xFFF << 16) | 0x10)
            .unwrap();
        adv(&clock, 128 * 0x0C);
        assert_eq!(wdt.counter(), 0x04);
        wdt.write_u32(WDT_CR, 1).unwrap(); // no key: ignored
        assert_eq!(wdt.counter(), 0x04);
        wdt.write_u32(WDT_CR, 0xA500_0001).unwrap();
        assert_eq!(wdt.counter(), 0x10);
        adv(&clock, 128 * 0x10);
        assert!(wdt.take_reset_request());
    }

    #[test]
    fn wdt_window_error_and_interrupt_without_reset() {
        let (mut wdt, clock) = wdt_at(32_768);
        // WDV 0x100, WDD 0x80, WDFIEN, no WDRSTEN.
        wdt.write_u32(WDT_MR, (0x80 << 16) | MR_WDFIEN | 0x100)
            .unwrap();
        adv(&clock, 128 * 0x10);
        wdt.write_u32(WDT_CR, 0xA500_0001).unwrap();
        assert!(!wdt.take_reset_request());
        assert!(wdt.tick_elapsed(1).irq);
        assert_eq!(wdt.read_u32(WDT_SR).unwrap(), SR_WDERR);
        assert!(!wdt.tick_elapsed(1).irq, "SR read releases the line");
        // Inside the window the restart is accepted.
        adv(&clock, 128 * 0x80);
        wdt.write_u32(WDT_CR, 0xA500_0001).unwrap();
        assert_eq!(wdt.read_u32(WDT_SR).unwrap(), 0);
        assert_eq!(wdt.counter(), 0x100);
    }

    #[test]
    fn rstc_software_reset_and_reset_type() {
        let mut rstc = Sam4sRstc::new();
        assert_eq!(rstc.read_u32(RSTC_SR).unwrap(), SR_NRSTL);
        rstc.write_u32(RSTC_CR, CR_PROCRST).unwrap(); // no key
        assert!(!rstc.take_reset_request());
        rstc.write_u32(RSTC_CR, 0xA500_0005).unwrap();
        assert_eq!(
            rstc.latched_reset_cause(),
            Some(PeripheralResetCause::System)
        );
        assert!(rstc.take_reset_request());
        rstc.on_peripheral_reset(PeripheralResetCause::System);
        assert_eq!(rstc.read_u32(RSTC_SR).unwrap() >> 8 & 7, RSTTYP_SOFTWARE);
        rstc.on_peripheral_reset(PeripheralResetCause::Watchdog);
        assert_eq!(rstc.read_u32(RSTC_SR).unwrap() >> 8 & 7, RSTTYP_WATCHDOG);
        rstc.write_u32(RSTC_MR, 0xA500_0010).unwrap();
        assert_eq!(rstc.read_u32(RSTC_MR).unwrap(), 0x10);
    }

    #[test]
    fn eefc_getd_lock_bits_and_gpnvm() {
        let mut eefc = Sam4sEefc::new(1024 * 1024);
        eefc.write_u32(EEFC_FMR, 0x0000_0500).unwrap();
        assert_eq!(eefc.read_u32(EEFC_FMR).unwrap(), 0x500);
        assert_eq!(eefc.read_u32(EEFC_FSR).unwrap(), FSR_FRDY);
        eefc.write_u32(EEFC_FCR, 0x5A00_0000).unwrap();
        let d: Vec<u32> = (0..8).map(|_| eefc.read_u32(EEFC_FRR).unwrap()).collect();
        assert_eq!(d, [0, 0x10_0000, 512, 1, 0x10_0000, 128, 8192, 8192]);
        // Lock the region holding page 20 (region 1), read it back.
        eefc.write_u32(EEFC_FCR, 0x5A00_0000 | (20 << 8) | 0x08)
            .unwrap();
        eefc.write_u32(EEFC_FCR, 0x5A00_000A).unwrap();
        assert_eq!(eefc.read_u32(EEFC_FRR).unwrap(), 0b10);
        eefc.write_u32(EEFC_FCR, 0x5A00_000D).unwrap();
        assert_eq!(
            eefc.read_u32(EEFC_FRR).unwrap(),
            0b10,
            "GPNVM1: boot from flash"
        );
        eefc.write_u32(EEFC_FCR, 0x5A00_000B).unwrap();
        eefc.write_u32(EEFC_FCR, 0x5A00_000D).unwrap();
        assert_eq!(eefc.read_u32(EEFC_FRR).unwrap(), 0b11);
        // Bad key: FCMDE, cleared by the read.
        eefc.write_u32(EEFC_FCR, 0x0000_0000).unwrap();
        assert_eq!(eefc.read_u32(EEFC_FSR).unwrap(), FSR_FRDY | FSR_FCMDE);
        assert_eq!(eefc.read_u32(EEFC_FSR).unwrap(), FSR_FRDY);
    }

    #[test]
    fn eefc_refuses_program_and_erase() {
        let mut eefc = Sam4sEefc::new(128 * 1024);
        assert!(eefc.write_u32(EEFC_FCR, 0x5A00_0003).is_err());
        assert!(eefc.write_u32(EEFC_FCR, 0x5A00_0005).is_err());
    }
}
