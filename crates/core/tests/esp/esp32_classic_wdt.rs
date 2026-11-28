// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Classic ESP32 Main System Watchdog (TIMG0 MWDT), proven by register-level
//! Xtensa firmware.
//!
//! Demand rank: the ESP32 family is #1 in the JetBrains State of Embedded 2025
//! MCU survey (38 % of respondents). This is depth on a part the engine
//! already boots, not a new part: before this change the classic TIMG
//! watchdog answered every feed and never latched, so a firmware that starved
//! the watchdog ran to completion instead of reporting a timeout.
//!
//! Firmware: `crates/core/tests/fixtures/esp32-classic-wdt/` (two hand-written
//! Xtensa programs, rebuilt by `build.sh`, committed as ELFs):
//!
//! * `wdt_starve.S` arms TIMG0's watchdog with a 16-cycle stage-0 hold and an
//!   interrupt action, masks maskable CPU interrupts, polls `INT_RAW_TIMERS`
//!   bit 2, then writes `INT_CLR_TIMERS` bit 2 and re-reads.
//! * `wdt_feed.S` arms the same watchdog and feeds it on every loop iteration
//!   for 20,000 iterations — thousands of scheduler-mode holds, and more CPU
//!   cycles than the walk-mode hold — then reads bit 2, which must still be
//!   clear.
//!
//! Console is UART0 (`debug_uart: uart0`), one byte per store, as on the
//! dev-board USB-UART bridge.

use crate::common;
use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use std::time::Duration;

const FIXTURE_DIR: &str = "crates/core/tests/fixtures/esp32-classic-wdt";
/// Classic ESP32-DevKitM-1 (ESP32-MINI-1, same classic chip descriptor as
/// every ESP32 board in the tree), UART0 console.
const SYSTEM: &str = "configs/systems/esp32-devkitm-1.yaml";

fn run(fixture: &str, stop: &str) -> String {
    let (chip, manifest) = common::system(SYSTEM);
    assert_eq!(chip.name, "esp32", "{SYSTEM}: not the classic chip");
    let fw = common::committed(&format!("{FIXTURE_DIR}/{fixture}"));
    let mut s = Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &BlobMap::new(),
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .expect("open session");
    let r = s.expect(stop, Duration::from_secs(10));
    let text = s.uart_transcript();
    r.unwrap_or_else(|e| panic!("{e}\n--- console ---\n{text}"));
    text
}

/// A starved watchdog reaches stage 0's interrupt action, latches
/// `INT_RAW_TIMERS.WDT_INT_RAW` (bit 2), and `INT_CLR_TIMERS` bit 2 clears it.
#[test]
fn starved_classic_wdt_latches_and_int_clr_clears() {
    let text = run("wdt_starve.elf", "WDT_DONE\r\n");
    assert_eq!(
        text, "WDT_ARM\r\nWDT_FIRED\r\nWDT_CLEARED\r\nWDT_DONE\r\n",
        "starved MWDT must latch bit 2, and INT_CLR bit 2 must clear it"
    );
}

/// A fed watchdog never reaches stage 0's expiry: 20,000 feed writes cross
/// thousands of scheduler-mode holds of the same 16-cycle configuration the
/// starve fixture times out under.
#[test]
fn fed_classic_wdt_never_latches() {
    let text = run("wdt_feed.elf", "WDT_FED_OK\r\n");
    assert_eq!(
        text, "WDT_ARM\r\nWDT_FED_OK\r\n",
        "a fed MWDT must not latch; WDT_LATCHED means the feed did not re-arm the chain"
    );
}
