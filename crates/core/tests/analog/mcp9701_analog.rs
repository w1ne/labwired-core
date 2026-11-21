// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use labwired_core::peripherals::components::declarative_analog::DeclarativeAnalogKit;
use labwired_core::sim_input::SimInput;

// Integration coverage: descriptor lookup -> kit construction -> live SimInput -> ADC millivolts.
// DS20001942G p.2: V_out = 400 mV + 19.5 mV/°C * T over -10..+125 °C.
#[test]
fn mcp9701_tracks_the_datasheet_transfer_function() {
    let yaml =
        labwired_config::embedded_device_yaml("mcp9701").expect("mcp9701 descriptor is embedded");
    let kit = DeclarativeAnalogKit::from_yaml(yaml).expect("mcp9701 descriptor builds");
    let mut device = kit.build(0).expect("mcp9701 device attaches");

    assert_eq!(
        device.output_mv(),
        887,
        "default 25 °C = 887.5 mV, truncated"
    );
    for (temperature_c, expected_mv) in [
        (-10.0, 205),
        (0.0, 400),
        (25.0, 887),
        (100.0, 2350),
        (125.0, 2837),
    ] {
        device
            .set_input("temperature", temperature_c)
            .expect("in-range temperature stimulus is accepted");
        assert_eq!(
            device.output_mv(),
            expected_mv,
            "temperature={temperature_c} °C"
        );
    }

    assert!(device.set_input("temperature", 126.0).is_err());
    assert!(device.set_input("temperature", -11.0).is_err());
    assert!(device.set_input("unknown", 25.0).is_err());
}
