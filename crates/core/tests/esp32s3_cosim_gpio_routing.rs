// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Regressions for the S3 pull-up circuit's pre-boot UnknownPad failure.
//! Exercise the real GPIO model, routing and MMIO, including bank-1 aliases.
use labwired_config::{CosimAdapter, CosimModelConfig};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSignalValue, CosimSignals, SignalRouter};
use labwired_core::peripherals::esp32s3::gpio::Esp32s3Gpio;
use labwired_core::Bus;

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
