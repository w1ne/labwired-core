// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The RP2350 moves the clock/reset sub-blocks to new bases (CLOCKS
//! 0x40010000, RESETS 0x40020000, XOSC 0x40048000, PLL_SYS 0x40050000,
//! PLL_USB 0x40058000, ROSC 0x400E8000 — rp2350 addressmap.h). The shared
//! clock/reset model must serve both maps behind a profile.

use labwired_core::peripherals::rp2040_clocks::{ClockResetProfile, Rp2040ClockReset};
use labwired_core::Peripheral;

#[test]
fn rp2350_profile_places_resets_at_the_rp2350_base() {
    // Window base = CLOCKS at 0x40010000; RESETS is +0x10000 into the window,
    // RESET_DONE at +0x8 within RESETS.
    let mut clk = Rp2040ClockReset::with_profile(0x4001_0000, ClockResetProfile::Rp2350);
    // Power-on: every peripheral held in reset → nothing done.
    assert_eq!(clk.read_u32(0x1_0008).unwrap(), 0);
    // Release everything → done everywhere.
    clk.write_u32(0x1_0000, 0).unwrap();
    assert_ne!(clk.read_u32(0x1_0008).unwrap(), 0);
}

#[test]
fn rp2350_profile_reports_oscillator_and_pll_ready_at_rp2350_offsets() {
    let clk = Rp2040ClockReset::with_profile(0x4001_0000, ClockResetProfile::Rp2350);
    // XOSC at 0x40048000 (offset 0x38000), STATUS at +0x4, STABLE = bit 31.
    assert_ne!(clk.read_u32(0x3_8004).unwrap() & (1 << 31), 0);
    // PLL_SYS at 0x40050000 (offset 0x40000), CS at +0x0, LOCK = bit 31.
    assert_ne!(clk.read_u32(0x4_0000).unwrap() & (1 << 31), 0);
    // PLL_USB at 0x40058000 (offset 0x48000).
    assert_ne!(clk.read_u32(0x4_8000).unwrap() & (1 << 31), 0);
}

#[test]
fn rp2040_profile_is_unchanged() {
    let mut clk = Rp2040ClockReset::new(0x4000_8000);
    // RP2040: RESETS at 0x4000C000 → offset 0x4000; RESET_DONE at +0x8.
    assert_eq!(clk.read_u32(0x4008).unwrap(), 0);
    clk.write_u32(0x4000, 0).unwrap();
    assert_ne!(clk.read_u32(0x4008).unwrap(), 0);
    // XOSC at 0x40024000 → offset 0x1C004, STABLE bit set.
    assert_ne!(clk.read_u32(0x1_c004).unwrap() & (1 << 31), 0);
}
