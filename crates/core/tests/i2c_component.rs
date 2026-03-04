// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::peripherals::components::Mpu6050;
use labwired_core::peripherals::i2c::I2c;
use labwired_core::Peripheral;

#[test]
fn test_mpu6050_who_am_i() {
    let mut i2c = I2c::new();
    let mpu = Mpu6050::new(0x68);
    i2c.attach(Box::new(mpu));

    // 1. START
    i2c.write(0x00, 0x01).unwrap(); // PE (Peripheral Enable)
    i2c.write(0x01, 0x01).unwrap(); // CR1: SB (Start Bit)
    for _ in 0..10 {
        i2c.tick();
    }
    // Check SB is set
    assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0); 

    // 2. Address (0x68 << 1 = 0xD0, write mode: LSB=0)
    i2c.write(0x10, 0xD0).unwrap(); // DR
    for _ in 0..20 {
        i2c.tick();
    }
    // Wait, AddressPending transition clears SB, sets ADDR
    // ADDR should be set
    assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0); 

    // 3. Register Address (0x75 = WHO_AM_I)
    i2c.write(0x10, 0x75).unwrap(); // DR
    for _ in 0..20 {
        i2c.tick();
    }
    // TxE should be set, but the master sent the register address to the component
    assert_ne!(i2c.peek(0x14).unwrap() & 0x80, 0); 

    // 4. Repeated START
    i2c.write(0x01, 0x01).unwrap(); // CR1: SB
    for _ in 0..10 {
        i2c.tick();
    }
    
    // 5. Address (0x68 << 1 = 0xD0, read mode: LSB=1 -> 0xD1)
    i2c.write(0x10, 0xD1).unwrap(); // DR
    for _ in 0..40 {
        i2c.tick();
    }
    
    // Wait, in read mode, after Address phase (which sets ADDR), the master starts reading data instantly?
    // In our simplified model, when is_reading is true, we fetch the data into DR and set RXNE
    // So RXNE should be set now!
    let sr1 = i2c.peek(0x14).unwrap();
    assert_ne!(sr1 & 0x40, 0, "RXNE should be set after address phase in read mode");
    
    // Read the data
    let data = i2c.read(0x10).unwrap();
    assert_eq!(data, 0x68, "WHO_AM_I should be 0x68");
    
    // 6. STOP
    i2c.write(0x01, 0x02).unwrap(); // CR1: STOP
    for _ in 0..10 {
        i2c.tick();
    }
}
