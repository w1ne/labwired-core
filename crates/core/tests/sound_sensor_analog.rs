// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use labwired_core::peripherals::components::declarative_analog::DeclarativeAnalogKit;
use labwired_core::sim_input::SimInput;

// Integration coverage: descriptor lookup -> kit construction -> live SimInput -> ADC millivolts.
#[test]
fn sound_sensor_tracks_the_declared_sound_envelope() {
    let yaml = labwired_config::embedded_device_yaml("sound-sensor")
        .expect("sound-sensor descriptor is embedded");
    let kit = DeclarativeAnalogKit::from_yaml(yaml).expect("sound-sensor descriptor builds");
    let mut device = kit.build(0).expect("sound-sensor device attaches");

    for (sound_pct, expected_mv) in [
        (0.0, 0),
        (25.0, 825),
        (50.0, 1650),
        (75.0, 2475),
        (100.0, 3300),
    ] {
        device
            .set_input("sound", sound_pct)
            .expect("in-range sound stimulus is accepted");
        assert_eq!(device.output_mv(), expected_mv, "sound={sound_pct}%");
    }

    assert!(device.set_input("sound", 101.0).is_err());
    assert!(device.set_input("unknown", 50.0).is_err());
}
