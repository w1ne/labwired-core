// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Generic MNA netlists drive the nRF54L15-DK application-core SAADC.

mod common;

#[test]
fn nrf54l15_dk_ntc_and_diode_netlists_reach_distinct_saadc_channels() {
    common::nrf54l_analog::assert_ntc_and_diode_saadc_profile(
        "configs/systems/nrf54l15dk.yaml",
        "examples/nrf54l15-dk/analog-ntc-diode-system.yaml",
        "nrf54l15dk",
        &[],
        &[
            ("led0", "gpio2", 9),
            ("led1", "gpio1", 10),
            ("led2", "gpio2", 7),
            ("led3", "gpio1", 14),
            ("button0", "gpio1", 13),
            ("button1", "gpio1", 9),
            ("button2", "gpio1", 8),
            ("button3", "gpio0", 4),
        ],
    );
}
