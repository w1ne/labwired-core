// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! IWDG (independent watchdog) — STM32 layout, identical across families.
//!
//! Four registers: KR (key), PR (prescaler), RLR (reload), SR (status),
//! WINR (windowed mode). WCH's CH32V003 IWDG is the same block under other
//! names (CTLR/PSCR/RLDR/STATR, CH32V003 RM §9.2), without WINR.
//!
//! Two modes:
//!
//! - Default (no `lsi_hz`): the writes are latched and the watchdog never
//!   counts or resets. Survival tests on STM32 boards that start the IWDG
//!   and never feed it rely on that.
//! - Counting (`config: { lsi_hz: N }`): KR = 0xCCCC starts a 12-bit
//!   down-counter from 0xFFF, clocked at LSI / (4 << PR) (PR 6 and 7 are
//!   both /256). KR = 0xAAAA reloads it from RLR. When it reaches 0 the
//!   model latches a CPU reset request with cause `Watchdog`; the machine
//!   reboots the core at the next instruction boundary. Once started it
//!   cannot be stopped, only reset. The system reset stops it again (the
//!   "hardware watchdog" option byte is not modelled). Core cycles come
//!   from the bus and are converted with the chip's `cpu_hz`, so the model
//!   assumes the core runs at `cpu_hz` for the whole run. The PVU/RVU busy
//!   flags in SR stay 0, and prescaler/reload updates apply at once.
//!
//! Reset values verified against NUCLEO-L476RG silicon:
//!   KR = 0, PR = 0, RLR = 0x0FFF (default reload = max 12-bit value),
//!   SR = 0.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Iwdg {
    kr: u32,
    pr: u32,
    rlr: u32,
    sr: u32,
    winr: u32,
    /// PR/RLR are write-protected: a write only takes effect after KR has
    /// received the 0x5555 unlock code (RM0008 §19.4). Set by that code,
    /// cleared by any other KR write. Transient state — kept out of the
    /// snapshot so the JSON format is unchanged.
    #[serde(skip, default)]
    write_access: bool,
    /// Counting state, present only when the descriptor gives `lsi_hz`.
    /// Skipped when absent so the inert model's snapshot JSON is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    countdown: Option<Countdown>,
}

/// The running watchdog of the counting mode.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Countdown {
    lsi_hz: u64,
    #[serde(skip, default)]
    cpu_hz: u64,
    running: bool,
    counter: u32,
    /// Elapsed core cycles times `lsi_hz` not yet worth a counter step.
    phase: u64,
    #[serde(skip, default)]
    reset_pending: bool,
}

/// KR codes.
const KEY_UNLOCK: u32 = 0x5555;
const KEY_RELOAD: u32 = 0xAAAA;
const KEY_START: u32 = 0xCCCC;
const COUNTER_RESET: u32 = 0xFFF;

impl Iwdg {
    pub fn new() -> Self {
        Self {
            kr: 0,
            pr: 0,
            rlr: 0x0000_0FFF,
            sr: 0,
            winr: 0x0000_0FFF,
            write_access: false,
            countdown: None,
        }
    }

    /// Counting mode: the watchdog counts down at `lsi_hz` / prescaler once
    /// started and requests a CPU reset when it runs out.
    pub fn with_lsi_hz(mut self, lsi_hz: u64) -> Self {
        self.countdown = Some(Countdown {
            lsi_hz,
            cpu_hz: 0,
            running: false,
            counter: COUNTER_RESET,
            phase: 0,
            reset_pending: false,
        });
        self
    }

    /// LSI cycles per counter step: 4 << PR, capped at /256.
    fn divider(&self) -> u64 {
        4u64 << self.pr.min(6)
    }

    /// Counter value of the counting mode; `None` in the default mode.
    pub fn counter(&self) -> Option<u32> {
        self.countdown.as_ref().map(|c| c.counter)
    }

    fn advance(&mut self, cycles: u64) {
        let divider = self.divider();
        let Some(c) = self.countdown.as_mut() else {
            return;
        };
        if !c.running || c.reset_pending || c.cpu_hz == 0 {
            return;
        }
        // One counter step every divider * cpu_hz / lsi_hz core cycles,
        // kept exact by accumulating cycles * lsi_hz.
        let step = divider * c.cpu_hz;
        c.phase += cycles * c.lsi_hz;
        let steps = c.phase / step;
        c.phase %= step;
        if steps >= u64::from(c.counter) {
            c.counter = 0;
            c.reset_pending = true;
        } else {
            c.counter -= steps as u32;
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.kr,
            0x04 => self.pr,
            0x08 => self.rlr,
            0x0C => self.sr,
            0x10 => self.winr,
            _ => {
                crate::census_reg!("iwdg:Iwdg", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // KR is write-only. 0x5555 unlocks PR/RLR write access; any other
            // code (0xAAAA reload, 0xCCCC start, …) re-protects them. We latch
            // the value for readback observability.
            0x00 => {
                let key = value & 0xFFFF;
                self.kr = key;
                self.write_access = key == KEY_UNLOCK;
                let rlr = self.rlr;
                if let Some(c) = self.countdown.as_mut() {
                    match key {
                        KEY_START if !c.running => {
                            c.running = true;
                            c.counter = COUNTER_RESET;
                            c.phase = 0;
                        }
                        KEY_RELOAD if c.running => c.counter = rlr,
                        _ => {}
                    }
                }
            }
            // PR/RLR are write-protected — a write without the 0x5555 unlock is
            // dropped (silicon-verified on the bench F103). Note: real silicon
            // additionally needs the IWDG clock domain running for the new value
            // to latch; this model stores it on unlock alone, which is the
            // useful behaviour for survival sims.
            0x04 if self.write_access => self.pr = value & 0x7,
            0x08 if self.write_access => self.rlr = value & 0xFFF,
            0x0C => {} // SR is read-only
            0x10 => self.winr = value & 0xFFF,
            _ => {
                crate::census_reg!("iwdg:Iwdg", offset, "write");
            }
        }
    }
}

impl Default for Iwdg {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Iwdg {
    // The default mode is an inert register bank; the counting mode must see
    // every elapsed cycle.
    fn needs_legacy_walk(&self) -> bool {
        self.countdown.is_some()
    }

    fn tick_elapsed(&mut self, cycles: u64) -> crate::PeripheralTickResult {
        self.advance(cycles);
        crate::PeripheralTickResult::default()
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        if let Some(c) = self.countdown.as_mut() {
            c.cpu_hz = hz;
        }
    }

    fn reset_request_capable(&self) -> bool {
        self.countdown.is_some()
    }

    fn take_reset_request(&mut self) -> bool {
        match self.countdown.as_mut() {
            Some(c) if c.reset_pending => {
                c.reset_pending = false;
                true
            }
            _ => false,
        }
    }

    fn latched_reset_cause(&self) -> Option<crate::PeripheralResetCause> {
        self.countdown
            .as_ref()
            .filter(|c| c.reset_pending)
            .map(|_| crate::PeripheralResetCause::Watchdog)
    }

    /// A system reset stops the watchdog and returns its registers to their
    /// power-on values; the counting configuration stays.
    fn on_system_reset(&mut self) {
        let countdown = self.countdown.take();
        *self = Self::new();
        if let Some(c) = countdown {
            *self = std::mem::take(self).with_lsi_hz(c.lsi_hz);
            self.attach_cpu_hz(c.cpu_hz);
        }
    }

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
    /// A word store is one register write, so KR sees the whole key at once
    /// rather than four byte-merged intermediates.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::Iwdg;
    use crate::Peripheral;

    #[test]
    fn pr_rlr_write_protected_until_key() {
        // Silicon-verified on the bench F103
        // (stm32f1_exec_oracle::iwdg_pr_rlr_write_protected_without_key).
        let mut w = Iwdg::new();
        // Without the 0x5555 unlock, PR/RLR writes are dropped → reset values.
        w.write_u32(0x04, 0x5).unwrap();
        w.write_u32(0x08, 0x123).unwrap();
        assert_eq!(w.read_u32(0x04).unwrap(), 0x0);
        assert_eq!(w.read_u32(0x08).unwrap(), 0xFFF);

        // After 0x5555, PR/RLR accept writes.
        w.write_u32(0x00, 0x5555).unwrap();
        w.write_u32(0x04, 0x5).unwrap();
        w.write_u32(0x08, 0x123).unwrap();
        assert_eq!(w.read_u32(0x04).unwrap(), 0x5);
        assert_eq!(w.read_u32(0x08).unwrap(), 0x123);

        // Any other KR code (e.g. 0xAAAA reload) re-protects them.
        w.write_u32(0x00, 0xAAAA).unwrap();
        w.write_u32(0x04, 0x2).unwrap();
        assert_eq!(
            w.read_u32(0x04).unwrap(),
            0x5,
            "re-protected after non-unlock KR"
        );
    }

    /// CH32V003 at 48 MHz with LSI 128 kHz: PR 5 (/128) is 1 kHz, so 1 ms
    /// is 48,000 core cycles per count.
    fn ch32v003() -> Iwdg {
        let mut w = Iwdg::new().with_lsi_hz(128_000);
        w.attach_cpu_hz(48_000_000);
        w
    }

    #[test]
    fn default_mode_never_counts_or_resets() {
        let mut w = Iwdg::new();
        w.write_u32(0x00, 0xCCCC).unwrap();
        w.tick_elapsed(u64::MAX / 1_000_000);
        assert!(!w.reset_request_capable());
        assert!(!w.take_reset_request());
        assert_eq!(w.counter(), None);
    }

    #[test]
    fn counts_from_0xfff_at_lsi_over_prescaler_and_resets_at_zero() {
        let mut w = ch32v003();
        w.write_u32(0x00, 0x5555).unwrap();
        w.write_u32(0x04, 5).unwrap();
        w.write_u32(0x00, 0xCCCC).unwrap();
        assert_eq!(w.counter(), Some(0xFFF));
        // One count per ms.
        w.tick_elapsed(48_000 * 10 + 47_999);
        assert_eq!(w.counter(), Some(0xFFF - 10));
        w.tick_elapsed(1);
        assert_eq!(w.counter(), Some(0xFFF - 11));
        assert!(!w.take_reset_request());
        w.tick_elapsed(48_000 * (0xFFF - 11));
        assert_eq!(
            w.latched_reset_cause(),
            Some(crate::PeripheralResetCause::Watchdog)
        );
        assert!(w.take_reset_request());
        assert!(!w.take_reset_request(), "the latch clears once taken");
    }

    #[test]
    fn reload_key_refills_from_rlr_and_start_is_once() {
        let mut w = ch32v003();
        w.write_u32(0x00, 0x5555).unwrap();
        w.write_u32(0x04, 5).unwrap();
        w.write_u32(0x00, 0x5555).unwrap();
        w.write_u32(0x08, 100).unwrap();
        // Reload before start does nothing.
        w.write_u32(0x00, 0xAAAA).unwrap();
        w.write_u32(0x00, 0xCCCC).unwrap();
        w.tick_elapsed(48_000 * 50);
        assert_eq!(w.counter(), Some(0xFFF - 50));
        w.write_u32(0x00, 0xAAAA).unwrap();
        assert_eq!(w.counter(), Some(100));
        // A second start does not restart the counter from 0xFFF.
        w.write_u32(0x00, 0xCCCC).unwrap();
        assert_eq!(w.counter(), Some(100));
        w.tick_elapsed(48_000 * 100);
        assert!(w.take_reset_request());
    }

    #[test]
    fn system_reset_stops_the_watchdog_and_keeps_counting_mode() {
        let mut w = ch32v003();
        w.write_u32(0x00, 0xCCCC).unwrap();
        w.tick_elapsed(48_000_000);
        w.on_system_reset();
        assert_eq!(w.counter(), Some(0xFFF));
        assert_eq!(w.read_u32(0x04).unwrap(), 0);
        w.tick_elapsed(48_000_000 * 100);
        assert!(!w.take_reset_request(), "stopped after the system reset");
        assert!(w.reset_request_capable());
        // Prescaler 0 is /4: 32 kHz, 1500 core cycles per count.
        w.write_u32(0x00, 0xCCCC).unwrap();
        w.tick_elapsed(1500 * 0xFFF);
        assert!(w.take_reset_request());
    }
}
