// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//! External-device GPIO input routes must reach both ESP32-S3 GPIO banks.
//! Offsets: ESP-IDF components/soc/esp32s3/register/soc/gpio_reg.h.
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::gpio::Esp32s3Gpio;
use labwired_core::Bus;
const BASE: u64 = 0x6000_4000;
fn bus() -> SystemBus {
    let mut bus = SystemBus::new();
    bus.add_peripheral("gpio", BASE, 0x1000, None, Box::new(Esp32s3Gpio::new()));
    bus
}
#[test]
fn s3_external_inputs_resolve_and_drive_both_banks_without_aliasing() {
    for pin in [0u8, 4, 8, 21, 26, 31, 32, 40, 48] {
        let mut b = bus();
        let label = format!("GPIO{pin}");
        let (offset, bit) = if pin < 32 {
            (0x3c, pin)
        } else {
            (0x40, pin - 32)
        };
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&b, &label),
            Some((BASE + offset, bit)),
            "{label}"
        );
        assert!(
            SystemBus::drive_pin_input(&mut b, &label, true),
            "{label} high"
        );
        assert_ne!(
            b.read_u32(BASE + offset).unwrap() & (1u32 << bit),
            0,
            "{label} read high"
        );
        if pin >= 32 {
            assert_eq!(
                b.read_u32(BASE + 0x3c).unwrap() & (1u32 << bit),
                0,
                "high-bank input must not alias low bank"
            );
        }
        assert!(
            SystemBus::drive_pin_input(&mut b, &label, false),
            "{label} low"
        );
        assert_eq!(
            b.read_u32(BASE + offset).unwrap() & (1u32 << bit),
            0,
            "{label} read low"
        );
    }
}
#[test]
fn s3_input_aliases_and_invalid_pad_are_explicit() {
    let mut b = bus();
    for label in ["GPIO4", "gpio4", "IO4", "4"] {
        assert_eq!(
            SystemBus::resolve_pin_idr_pub(&b, label),
            Some((BASE + 0x3c, 4))
        );
    }
    assert_eq!(SystemBus::resolve_pin_idr_pub(&b, "GPIO49"), None);
    assert!(!SystemBus::drive_pin_input(&mut b, "GPIO49", true));
}
