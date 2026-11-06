// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the nRF54L15 smart-ring profile's SAADC.

mod common;

#[test]
fn smart_ring_ntc_and_diode_netlists_reach_distinct_saadc_channels() {
    common::nrf54l_analog::assert_ntc_and_diode_saadc_profile(
        "configs/systems/smart-ring.yaml",
        "examples/nrf54l15-smart-ring/analog-ntc-diode-system.yaml",
        "smart-ring",
        &["imu", "ppg", "temp", "haptic"],
        &[
            ("touch", "gpio1", 13),
            ("motor_enable", "gpio1", 11),
            ("charger_detect", "gpio1", 12),
        ],
    );
}
