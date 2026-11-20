//! ESP32-C3 TRM GPIO edge/status and interrupt-matrix source contract.
use labwired_core::peripherals::esp32c3::gpio::Esp32c3Gpio;
use labwired_core::Peripheral;

#[test]
fn configured_gpio_edges_latch_raw_status_and_gate_the_cpu_irq() {
    let mut gpio = Esp32c3Gpio::new();
    gpio.set_gpio_input(7, true);
    gpio.write_u32(0x90, 2 << 7).unwrap(); // falling edge, CPU interrupt masked
    gpio.set_gpio_input(7, false);
    assert_eq!(gpio.read_u32(0x44).unwrap(), 1 << 7);
    assert_eq!(gpio.read_u32(0x5c).unwrap(), 0);
    assert!(gpio.matrix_irq_sources().is_empty());
    gpio.write_u32(0x90, (2 << 7) | (1 << 13)).unwrap();
    assert_eq!(gpio.read_u32(0x5c).unwrap(), 1 << 7);
    assert_eq!(gpio.matrix_irq_sources(), vec![16]);
    gpio.write(0x4c, 1 << 7).unwrap();
    assert!(gpio.matrix_irq_sources().is_empty());
    gpio.set_gpio_input(7, true); // rising edge is not selected
    assert_eq!(gpio.read_u32(0x44).unwrap(), 0);
    gpio.write_u32(0x90, (3 << 7) | (1 << 13)).unwrap();
    gpio.set_gpio_input(7, false);
    gpio.set_gpio_input(7, true);
    assert_eq!(gpio.read_u32(0x44).unwrap(), 1 << 7);
    gpio.write_u32(0x48, 1 << 10).unwrap(); // independent pending bit
    gpio.write(0x4c, 1 << 7).unwrap();
    assert_eq!(gpio.read_u32(0x44).unwrap(), 1 << 10, "selective W1C");
    assert_eq!(
        gpio.read_u32(0x5c).unwrap(),
        0,
        "raw pending does not bypass pin enable"
    );
}
