use labwired_core::peripherals::i2c::I2c;
use labwired_core::peripherals::spi::Spi;
use labwired_core::{bus::PeripheralEntry, bus::SystemBus, Bus};

#[test]
fn test_spi_fidelity_in_machine() {
    let mut bus = SystemBus::new();
    // Add SPI1 at 0x40013000
    bus.peripherals.push(PeripheralEntry {
        name: "SPI1".to_string(),
        base: 0x40013000,
        size: 0x400,
        irq: None,
        dev: Box::new(Spi::new()),
    });
    bus.refresh_peripheral_index();

    // 1. Enable SPI and set Baud Rate
    bus.write_u8(0x40013000, 0x48).unwrap(); // CR1: SPE=1, BR=1 (div 4)

    // 2. Start transfer by writing to DR
    bus.write_u8(0x4001300C, 0x55).unwrap();

    // Check BSY is set
    let sr = bus.read_u8(0x40013008).unwrap();
    assert_ne!(sr & 0x80, 0);

    // Step machine 32 cycles (8 bits * 4 divider)
    for _ in 0..32 {
        bus.tick_peripherals();
    }

    // Check BSY is cleared and TXE/RXNE set
    let sr = bus.read_u8(0x40013008).unwrap();
    assert_eq!(sr & 0x80, 0);
    assert_ne!(sr & 0x02, 0); // TXE
    assert_ne!(sr & 0x01, 0); // RXNE

    // Read DR
    let dr = bus.read_u8(0x4001300C).unwrap();
    assert_eq!(dr, 0x55);
}

#[test]
fn test_i2c_fidelity_in_machine() {
    let mut bus = SystemBus::new();
    bus.peripherals.push(PeripheralEntry {
        name: "I2C1".to_string(),
        base: 0x40005400,
        size: 0x400,
        irq: None,
        dev: Box::new(I2c::new()),
    });
    bus.refresh_peripheral_index();

    // 1. START
    bus.write_u8(0x40005401, 0x01).unwrap(); // CR1 START=1 (bit 8)
    for _ in 0..10 {
        bus.tick_peripherals();
    }
    assert_ne!(bus.read_u8(0x40005414).unwrap() & 0x01, 0); // SR1 SB=1

    // 2. Address
    bus.write_u8(0x40005410, 0xA0).unwrap(); // DR = 0xA0
    for _ in 0..20 {
        bus.tick_peripherals();
    }
    assert_eq!(bus.read_u8(0x40005414).unwrap() & 0x01, 0); // SB=0
    assert_ne!(bus.read_u8(0x40005414).unwrap() & 0x02, 0); // ADDR=1
}
