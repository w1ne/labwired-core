// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! APP_CPU bringup gate.
//!
//! Tracks the four conditions ESP32-S3 firmware satisfies to release the
//! second core: an entry address (set via `ets_set_appcpu_boot_addr`), and
//! three bits in `SYSTEM_CORE_1_CONTROL_0_REG` (0x600C_018C):
//! `RUNSTALL` (bit 1), `RESETTING` (bit 2), `CLKGATE_EN` (bit 0).
//!
//! Shared between the rom-thunk that captures the entry and the
//! `SystemStub` that watches the SYSTEM register writes via
//! `Arc<Mutex<CoreController>>`.

#[derive(Debug, Default, Clone, Copy)]
pub struct CoreController {
    pub appcpu_entry: Option<u32>,
    pub runstall: bool,
    pub reset_en: bool,
    pub clkgate_en: bool,
}

impl CoreController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_entry(&mut self, addr: u32) {
        self.appcpu_entry = Some(addr);
    }

    pub fn entry(&self) -> Option<u32> {
        self.appcpu_entry
    }

    pub fn set_runstall(&mut self, on: bool) {
        self.runstall = on;
    }

    pub fn set_reset_en(&mut self, on: bool) {
        self.reset_en = on;
    }

    pub fn set_clkgate_en(&mut self, on: bool) {
        self.clkgate_en = on;
    }

    /// True once the firmware has set an entry AND opened all three gates.
    /// On real silicon this corresponds to APP_CPU starting to fetch.
    pub fn is_app_cpu_released(&self) -> bool {
        self.appcpu_entry.is_some() && !self.runstall && !self.reset_en && self.clkgate_en
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_not_released() {
        let c = CoreController::new();
        assert!(!c.is_app_cpu_released());
        assert_eq!(c.entry(), None);
    }

    #[test]
    fn entry_alone_does_not_release() {
        let mut c = CoreController::new();
        c.set_entry(0x4080_1234);
        assert!(!c.is_app_cpu_released());
    }

    #[test]
    fn gates_alone_do_not_release() {
        let mut c = CoreController::new();
        c.set_clkgate_en(true);
        c.set_reset_en(false);
        c.set_runstall(false);
        assert!(!c.is_app_cpu_released());
    }

    #[test]
    fn entry_plus_open_gates_releases() {
        let mut c = CoreController::new();
        c.set_entry(0x4080_1234);
        c.set_clkgate_en(true);
        // reset_en / runstall default to false (their "released" values)
        assert!(c.is_app_cpu_released());
        assert_eq!(c.entry(), Some(0x4080_1234));
    }

    #[test]
    fn runstall_blocks_release() {
        let mut c = CoreController::new();
        c.set_entry(0x4080_1234);
        c.set_clkgate_en(true);
        c.set_runstall(true);
        assert!(!c.is_app_cpu_released());
        c.set_runstall(false);
        assert!(c.is_app_cpu_released());
    }

    #[test]
    fn reset_en_blocks_release() {
        let mut c = CoreController::new();
        c.set_entry(0x4080_1234);
        c.set_clkgate_en(true);
        c.set_reset_en(true);
        assert!(!c.is_app_cpu_released());
        c.set_reset_en(false);
        assert!(c.is_app_cpu_released());
    }

    #[test]
    fn clkgate_not_set_blocks_release() {
        let mut c = CoreController::new();
        c.set_entry(0x4080_1234);
        // runstall and reset_en default to false (their "released" values).
        // clkgate_en remains false (its "blocked" value).
        assert!(!c.is_app_cpu_released());
        c.set_clkgate_en(true);
        assert!(c.is_app_cpu_released());
    }
}
