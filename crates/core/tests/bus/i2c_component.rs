// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::peripherals::components::declarative_i2c::GenericI2cDevice;

/// The MPU6050 is a `configs/devices/mpu6050.yaml` descriptor; there is no
/// hand-written struct to construct any more.
fn mpu6050(address: u8) -> GenericI2cDevice {
    GenericI2cDevice::from_yaml(
        labwired_config::embedded_device_yaml("mpu6050").expect("mpu6050 is embedded"),
        address,
    )
    .expect("mpu6050.yaml builds")
}
use labwired_core::peripherals::i2c::I2c;
use labwired_core::Peripheral;

#[test]
fn test_mpu6050_who_am_i() {
    let mut i2c = I2c::new();
    let mpu = mpu6050(0x68);
    // Off-bus attach still goes through the mandatory-trace helper (no untraced
    // attach path); the throwaway trace is unused by this model-level test.
    let trace = labwired_core::bus::bus_trace::new_log();
    i2c.attach_traced("i2c1", &trace, Box::new(mpu));

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

    // ADDR is set and SCL is stretched until firmware clears it by reading
    // SR1 then SR2 (RM0008 §26.3.3); only then is the data byte clocked in.
    // ACK is off (single-byte read), so the byte is NACKed and is the last.
    let _ = i2c.read(0x14).unwrap();
    let _ = i2c.read(0x18).unwrap();
    for _ in 0..40 {
        i2c.tick();
    }
    let sr1 = i2c.peek(0x14).unwrap();
    assert_ne!(
        sr1 & 0x40,
        0,
        "RXNE should be set once ADDR is cleared and a byte time has passed"
    );

    // Read the data
    let data = i2c.read(0x10).unwrap();
    assert_eq!(data, 0x68, "WHO_AM_I should be 0x68");

    // 6. STOP
    i2c.write(0x01, 0x02).unwrap(); // CR1: STOP
    for _ in 0..10 {
        i2c.tick();
    }
}
