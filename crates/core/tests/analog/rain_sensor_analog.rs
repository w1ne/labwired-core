// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use labwired_core::peripherals::components::declarative_analog::DeclarativeAnalogKit;
use labwired_core::sim_input::SimInput;

// Integration coverage: descriptor lookup -> kit construction -> live SimInput -> ADC millivolts.
// Wetter plate → lower AO (ratiometric to the 5 V rail the curve is authored on).
#[test]
fn rain_sensor_ao_falls_as_rain_rises() {
    let yaml = labwired_config::embedded_device_yaml("rain-sensor")
        .expect("rain-sensor descriptor is embedded");
    let kit = DeclarativeAnalogKit::from_yaml(yaml).expect("rain-sensor descriptor builds");
    let mut device = kit.build(0).expect("rain-sensor device attaches");

    for (rain_pct, expected_mv) in [
        (0.0, 5000),
        (25.0, 3750),
        (50.0, 2500),
        (75.0, 1250),
        (100.0, 0),
    ] {
        device
            .set_input("rain", rain_pct)
            .expect("in-range rain stimulus is accepted");
        assert_eq!(device.output_mv(), expected_mv, "rain={rain_pct}%");
    }

    assert!(device.set_input("rain", 101.0).is_err());
    assert!(device.set_input("unknown", 50.0).is_err());
}
