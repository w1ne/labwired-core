# ESP32-C3 DevKit mixed-signal example

[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) connects a generic
Beta NTC divider and an unrelated Shockley diode load line to ADC1 channels 0
and 2. The shared analog adapter solves both netlists and routes their node
voltages into the modeled APB_SARADC inputs.

The integration test checks the temperature-dependent divider voltage, the
independent diode load-line voltage, channel isolation, and the ADC one-shot
result registers on the `esp32c3-devkit` system profile:

```sh
cargo test -p labwired-core --test cosim_esp32c3_analog esp32c3_devkit_ntc_and_diode_netlists_reach_adc1_channels
```

The ESP32-C3 descriptor has no `analog_pins` aliases, so the fixture names the
ADC peripheral and channels explicitly. This covers the simulated board route;
it does not claim silicon validation.
