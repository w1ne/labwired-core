// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Regressions for the S3 pull-up circuit's pre-boot UnknownPad failure.
//! Exercise the real GPIO model, routing and MMIO, including bank-1 aliases.
use labwired_config::{CosimAdapter, CosimModelConfig};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSignalValue, CosimSignals, SignalRouter};
use labwired_core::peripherals::esp32s3::gpio::Esp32s3Gpio;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3Opts,
};
use labwired_core::{AdvanceRequest, Bus, Machine};
use std::path::PathBuf;

const GPIO: u64 = 0x6000_4000;

fn bus() -> SystemBus {
    let mut bus = SystemBus::empty();
    bus.add_peripheral("gpio", GPIO, 0x800, None, Box::new(Esp32s3Gpio::new()));
    bus.refresh_peripheral_index();
    bus
}

fn routes(pad: u8) -> CosimModelConfig {
    CosimModelConfig {
        id: "s3_pin_routes".into(),
        adapter: CosimAdapter::Mock,
        model: None,
        step_ns: 100_000,
        inputs: ["gpio", "gpio_in", "gpio_pad", "gpio_output", "gpio_driven"]
            .into_iter()
            .map(|kind| (kind.into(), format!("board.{kind}.gpio{pad}")))
            .collect(),
        outputs: [("pin".into(), format!("board.gpio_in.gpio{pad}"))].into(),
        config: Default::default(),
    }
}

#[test]
fn s3_input_resolver_uses_the_correct_bank_and_register_bit() {
    let mut bus = bus();
    for (pin, addr, bit) in [
        ("GPIO0", GPIO + 0x3c, 0),
        ("GPIO31", GPIO + 0x3c, 31),
        ("GPIO32", GPIO + 0x40, 0),
        ("GPIO8", GPIO + 0x3c, 8),
        ("gpio9", GPIO + 0x3c, 9),
        ("IO40", GPIO + 0x40, 8),
        ("48", GPIO + 0x40, 16),
    ] {
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&bus, pin),
            Some((addr, bit)),
            "{pin}"
        );
        assert!(
            SystemBus::drive_pin_input(&mut bus, pin, true),
            "drive {pin}"
        );
        assert_ne!(bus.read_u32(addr).unwrap() & (1 << bit), 0, "read {pin}");
        assert!(SystemBus::drive_pin_input(&mut bus, pin, false));
        assert_eq!(bus.read_u32(addr).unwrap() & (1 << bit), 0);
    }
    assert_eq!(SystemBus::resolve_pin_idr_pub(&bus, "GPIO49"), None);
}

#[test]
fn s3_low_and_high_bank_cosim_routes_do_not_alias() {
    for pad in [0u8, 8, 9, 31, 32, 40, 48] {
        let mut bus = bus();
        let (mut router, errors) = SignalRouter::bind(&[routes(pad)], &bus);
        assert!(errors.is_empty(), "GPIO{pad} failed to bind: {errors:?}");
        let input = format!("board.gpio_in.gpio{pad}");
        let mut signals = CosimSignals::from([(input.clone(), CosimSignalValue::Bool(true))]);
        assert!(router.apply(&mut bus, &signals, 0).is_empty());
        router.sample(&bus, &mut signals);
        assert_eq!(signals[&input], CosimSignalValue::Bool(true));
        assert_eq!(
            signals[&format!("board.gpio_pad.gpio{pad}")],
            CosimSignalValue::Bool(true)
        );
        assert_eq!(
            signals[&format!("board.gpio_output.gpio{pad}")],
            CosimSignalValue::Bool(false)
        );
        assert_eq!(
            signals[&format!("board.gpio_driven.gpio{pad}")],
            CosimSignalValue::Bool(false)
        );
        if pad >= 32 {
            assert_eq!(
                bus.read_u32(GPIO + 0x3c).unwrap() & (1 << (pad - 32)),
                0,
                "bank-1 input must not change its bank-0 alias"
            );
        }
        // Also prove that output readback uses the absolute pad, not bank-local bit.
        let bit = pad % 32;
        let (out, enable) = if pad >= 32 {
            (0x10, 0x2c)
        } else {
            (0x04, 0x20)
        };
        bus.write_u32(GPIO + out, 1 << bit).unwrap();
        bus.write_u32(GPIO + enable, 1 << bit).unwrap();
        router.sample(&bus, &mut signals);
        assert_eq!(
            signals[&format!("board.gpio.gpio{pad}")],
            CosimSignalValue::Bool(true)
        );
        assert_eq!(
            signals[&format!("board.gpio_output.gpio{pad}")],
            CosimSignalValue::Bool(true)
        );
        assert_eq!(
            signals[&format!("board.gpio_driven.gpio{pad}")],
            CosimSignalValue::Bool(true)
        );
    }
}

/// The production manifest attachment path binds HC-SR04 TRIG on GPIO4 and
/// ECHO on GPIO5. A committed S3 firmware fixture raises TRIG; the resulting
/// sensor pulse must reach the GPIO input register exposed to firmware.
#[test]
fn s3_hcsr04_gpio4_trigger_drives_gpio5_echo_for_60cm() {
    const GPIO_IN: u64 = GPIO + 0x3c;
    const ECHO: u32 = 1 << 5;
    const DISTANCE_CM: u64 = 60;
    const CPU_HZ: u64 = 240_000_000;
    const SAMPLE_CYCLES: u64 = 10_000;
    const MAX_CYCLES: u64 = 3_000_000;

    let manifest: labwired_config::SystemManifest = serde_yaml::from_str(
        r#"
name: esp32s3-hcsr04-gpio-regression
chip: esp32s3
external_devices:
  - id: sonar
    type: hc-sr04
    connection: gpio
    config:
      trig_pin: GPIO4
      echo_pin: GPIO5
      cpu_hz: 240000000
      distance_cm: 60
"#,
    )
    .expect("manifest parses");
    let mut bus = SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    attach_esp32_external_devices(&mut bus, &manifest)
        .expect("HC-SR04 GPIO4/GPIO5 attachment must resolve on ESP32-S3");
    assert!(bus
        .list_inputs()
        .iter()
        .any(|(owner, channel)| { owner == "sonar" && channel.key == "distance" }));
    bus.set_input(Some("sonar"), "distance", DISTANCE_CM as f64)
        .expect("set the generic HC-SR04 distance stimulus");
    let sonar = bus
        .gpio_devices_of::<labwired_core::peripherals::components::declarative_gpio::DeclarativeGpioDevice>()
        .next()
        .expect("the generic HC-SR04 device is attached");
    assert_eq!(sonar.cpu_hz(), CPU_HZ);
    assert_eq!(sonar.input_value("distance"), Some(DISTANCE_CM as f64));
    // The board IO mux starts with weak pulls enabled. Establish the sensor's
    // documented idle-low ECHO level before firmware starts so that the first
    // observed high is the response edge, rather than the reset pull-up.
    assert!(SystemBus::drive_pin_input(&mut bus, "GPIO5", false));
    assert_eq!(bus.read_u32(GPIO_IN).unwrap() & ECHO, 0);

    let (trig_addr, trig_bit) = SystemBus::resolve_pin_odr_pub(&bus, "GPIO4")
        .expect("GPIO4 TRIG resolves to the S3 output register");
    let (echo_addr, echo_bit) = SystemBus::resolve_pin_idr_pub(&bus, "GPIO5")
        .expect("GPIO5 ECHO resolves to the S3 input register");
    assert_eq!((trig_addr, trig_bit), (GPIO + 0x04, 4));
    assert_eq!((echo_addr, echo_bit), (GPIO_IN, 5));

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let firmware = std::fs::read(root.join("tests/fixtures/tier1/esp32s3.elf"))
        .expect("read the committed ESP32-S3 firmware fixture");
    let mut cpu = wiring.cpu;
    labwired_core::boot::esp32s3::fast_boot(
        &firmware,
        &mut bus,
        &mut cpu,
        &labwired_core::boot::esp32s3::BootOpts {
            icache_backing: Some(wiring.icache_backing.clone()),
            dcache_backing: Some(wiring.dcache_backing.clone()),
            ..Default::default()
        },
    )
    .expect("fast boot the committed ESP32-S3 firmware");
    let mut machine = Machine::new(cpu, bus);

    let mut elapsed_cycles = 0;
    let mut echo_rise = None;
    let mut echo_fall = None;
    while elapsed_cycles < MAX_CYCLES {
        let report = machine
            .advance(AdvanceRequest::run(None).with_cycle_limit(SAMPLE_CYCLES))
            .expect("run the S3 firmware with the attached sensor");
        elapsed_cycles += report.elapsed_cycles;
        let high = machine.bus.read_u32(GPIO_IN).unwrap() & ECHO != 0;
        match (echo_rise, high) {
            (None, true) => echo_rise = Some(machine.bus.current_cycle),
            (Some(_), false) => {
                echo_fall = Some(machine.bus.current_cycle);
                break;
            }
            _ => {}
        }
    }

    let rise = echo_rise.expect("firmware's GPIO4 trigger must produce a GPIO5 echo");
    let fall = echo_fall.expect("the 60 cm ECHO pulse must return low");
    let measured_cycles = fall - rise;
    let expected_cycles = DISTANCE_CM * 58 * (CPU_HZ / 1_000_000);
    assert!(
        measured_cycles.abs_diff(expected_cycles) <= 25_000,
        "GPIO5 pulse should be 60 cm × 58 µs: measured {measured_cycles} cycles, expected {expected_cycles}"
    );
}
