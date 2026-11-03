// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end contract for the catalog `sound-sensor` input.
//!
//! The browser and MCP catalog have advertised a `sound` SimInput for this part
//! for a long time. Before the descriptor existed, however, no external device
//! was attached to the machine, so every hosted stimulus was rejected with
//! `no attached input device exposes channel 'sound'`. This test makes that
//! catalog/runtime mismatch impossible to reintroduce.

use labwired_core::peripherals::components::declarative_analog::{
    DeclarativeAnalogDevice, DeclarativeAnalogKit,
};
use labwired_core::sim_input::SimInput;

fn sound_sensor() -> DeclarativeAnalogDevice {
    let yaml = labwired_config::embedded_device_yaml("sound-sensor")
        .expect("sound-sensor descriptor must be embedded");
    DeclarativeAnalogKit::from_yaml(yaml)
        .expect("sound-sensor descriptor must parse")
        .build(0)
        .expect("sound-sensor device must attach to ADC channel 0")
}

#[test]
fn sound_stimulus_drives_the_adc_envelope() {
    let mut sensor = sound_sensor();

    assert_eq!(sensor.output_mv(), 0, "the documented quiet default is 0 mV");
    for (level, expected_mv) in [
        (25.0, 825u16),
        (50.0, 1650u16),
        (75.0, 2475u16),
        (100.0, 3300u16),
    ] {
        sensor
            .set_input("sound", level)
            .unwrap_or_else(|e| panic!("sound={level} must be accepted: {e:?}"));
        assert_eq!(sensor.output_mv(), expected_mv, "sound={level}");
    }
}

#[test]
fn sound_input_contract_is_named_and_bounded() {
    let mut sensor = sound_sensor();
    let channels = sensor.input_channels();

    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].key, "sound");
    assert_eq!(channels[0].min, 0.0);
    assert_eq!(channels[0].max, 100.0);

    assert!(sensor.set_input("sound", -1.0).is_err());
    assert!(sensor.set_input("sound", 101.0).is_err());
    assert!(sensor.set_input("amplitude", 50.0).is_err());
}
