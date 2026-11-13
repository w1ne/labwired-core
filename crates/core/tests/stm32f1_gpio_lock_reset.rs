use labwired_core::peripherals::gpio::{GpioPort, GpioRegisterLayout};
use labwired_core::Peripheral;

#[test]
fn system_reset_releases_f1_gpio_configuration_lock() {
    let mut gpio = GpioPort::new();
    gpio.write_u32(0x00, 0x4444_4443).unwrap();
    gpio.write_u32(0x18, 0x0001_0001).unwrap();
    gpio.write_u32(0x18, 0x0000_0001).unwrap();
    gpio.write_u32(0x18, 0x0001_0001).unwrap();
    assert_eq!(gpio.read_u32(0x18).unwrap(), 0x0001_0001);
    assert!(gpio.set_gpio_input(2, true));

    gpio.on_system_reset();
    assert_eq!(gpio.read_u32(0x18).unwrap(), 0);
    assert_eq!(gpio.read_u32(0x00).unwrap(), 0x4444_4444);
    assert_eq!(
        gpio.read_u32(0x08).unwrap() & 4,
        4,
        "external input survives reset"
    );
    gpio.write_u32(0x00, 0x4444_4441).unwrap();
    assert_eq!(gpio.read_u32(0x00).unwrap(), 0x4444_4441);
}

#[test]
fn f1_reset_change_does_not_reset_wch_gpio() {
    let mut gpio = GpioPort::new_with_layout(GpioRegisterLayout::Wch);
    gpio.write_u32(0x00, 0x0000_0002).unwrap();
    gpio.on_system_reset();
    assert_eq!(gpio.read_u32(0x00).unwrap(), 0x0000_0002);
}
