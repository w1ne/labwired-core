// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! IWDG (independent watchdog) — STM32 layout, identical across families.
//!
//! Four registers: KR (key), PR (prescaler), RLR (reload), SR (status),
//! WINR (windowed mode). Real silicon resets the chip if the watchdog
//! isn't kicked within the configured timeout — our simulator just
//! latches the writes and never resets, since survival tests need
//! deterministic completion.
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
    /// Opt-in expiry (chip yaml `reset_on_expiry: { lsi_hz: N }`): the
    /// counter runs from KR=0xCCCC and an elapsed timeout requests a machine
    /// reset. `None` keeps the historical latch-only model.
    #[serde(skip, default)]
    expiry: Option<IwdgExpiry>,
}

/// Runtime of an expiring IWDG. Times are in CPU cycles of the shared clock.
#[derive(Debug, Default)]
struct IwdgExpiry {
    lsi_hz: u64,
    cpu_hz: u64,
    running: bool,
    deadline: u64,
    clock: Option<crate::CycleClock>,
}

impl Iwdg {
    pub fn new() -> Self {
        Self {
            kr: 0,
            pr: 0,
            rlr: 0x0000_0FFF,
            sr: 0,
            winr: 0x0000_0FFF,
            write_access: false,
            expiry: None,
        }
    }

    /// Count down from the low-speed clock and reset the machine on expiry.
    /// `lsi_hz` is the nominal LSI/LICK frequency the descriptor declares;
    /// RC oscillator tolerance is not modelled.
    pub fn with_reset_on_expiry(mut self, lsi_hz: u64) -> Self {
        self.expiry = Some(IwdgExpiry {
            lsi_hz: lsi_hz.max(1),
            cpu_hz: 1,
            ..Default::default()
        });
        self
    }

    /// Reload the down-counter: timeout = (RLR + 1) * (4 << PR) / LSI.
    fn reload_counter(&mut self) {
        let (pr, rlr) = (self.pr.min(6), self.rlr);
        if let Some(e) = self.expiry.as_mut() {
            let lsi_ticks = u64::from(rlr + 1) * (4u64 << pr);
            let cycles = (lsi_ticks * e.cpu_hz).div_ceil(e.lsi_hz).max(1);
            let now = e.clock.as_ref().map_or(0, |c| c.now());
            e.deadline = now.saturating_add(cycles);
        }
    }

    fn expired(&self) -> bool {
        self.expiry
            .as_ref()
            .is_some_and(|e| e.running && e.clock.as_ref().map_or(0, |c| c.now()) >= e.deadline)
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
                self.kr = value & 0xFFFF;
                self.write_access = (value & 0xFFFF) == 0x5555;
                match value & 0xFFFF {
                    0xCCCC => {
                        if let Some(e) = self.expiry.as_mut() {
                            e.running = true;
                        }
                        self.reload_counter();
                    }
                    0xAAAA if self.expiry.as_ref().is_some_and(|e| e.running) => {
                        self.reload_counter()
                    }
                    _ => {}
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
    // Inert walk: stub register bank that does not count down today; tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
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
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn attach_cpu_hz(&mut self, hz: u64) {
        if let Some(e) = self.expiry.as_mut() {
            e.cpu_hz = hz.max(1);
        }
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        if let Some(e) = self.expiry.as_mut() {
            e.clock = Some(clock);
        }
    }
    fn reset_request_capable(&self) -> bool {
        self.expiry.is_some()
    }
    fn take_reset_request(&mut self) -> bool {
        let expired = self.expired();
        if expired {
            if let Some(e) = self.expiry.as_mut() {
                e.running = false;
            }
        }
        expired
    }
    fn latched_reset_cause(&self) -> Option<crate::PeripheralResetCause> {
        self.expired()
            .then_some(crate::PeripheralResetCause::Watchdog)
    }
    /// An expiring IWDG returns to its stopped power-on state on any reset
    /// (a running IWDG cannot otherwise be stopped by software).
    fn on_system_reset(&mut self) {
        if let Some(mut e) = self.expiry.take() {
            e.running = false;
            *self = Self {
                expiry: Some(e),
                ..Self::new()
            };
        }
    }
    fn on_node_reset(&mut self) {
        self.on_system_reset();
    }
    fn on_supply_reset(&mut self, _cause: crate::power::SupplyResetCause) {
        self.on_system_reset();
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

    #[test]
    fn latch_only_iwdg_never_requests_a_reset() {
        let mut w = Iwdg::new();
        assert!(!w.reset_request_capable());
        w.write_u32(0x00, 0xCCCC).unwrap();
        assert!(!w.take_reset_request());
    }

    #[test]
    fn expiring_iwdg_resets_after_the_programmed_timeout_unless_reloaded() {
        let clock = crate::CycleClock::default();
        let mut w = Iwdg::new().with_reset_on_expiry(40_000);
        w.attach_cpu_hz(120_000_000);
        w.attach_cycle_clock(clock.clone());
        assert!(w.reset_request_capable());
        w.write_u32(0x00, 0x5555).unwrap();
        w.write_u32(0x04, 0).unwrap(); // /4
        w.write_u32(0x08, 9).unwrap(); // 10 counts: 40 LSI ticks = 1 ms.
        w.write_u32(0x00, 0xCCCC).unwrap();
        clock.publish(119_999);
        assert!(!w.take_reset_request(), "1 ms is 120000 cycles");
        w.write_u32(0x00, 0xAAAA).unwrap(); // Reload restarts the window.
        clock.publish(239_998);
        assert!(!w.take_reset_request());
        clock.publish(239_999);
        assert_eq!(
            w.latched_reset_cause(),
            Some(crate::PeripheralResetCause::Watchdog)
        );
        assert!(w.take_reset_request());
        assert!(!w.take_reset_request(), "one request per expiry");
        w.on_system_reset();
        assert_eq!(w.read_u32(0x08).unwrap(), 0xFFF);
        assert!(w.reset_request_capable(), "expiry mode survives reset");
    }
}
