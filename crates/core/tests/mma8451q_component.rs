// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Model-level tests over the I2cDevice trait (pointer protocol, driven
//! directly, no bus master).

use labwired_core::peripherals::components::Mma8451q;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::sim_input::SimInput;

fn read_reg(dev: &mut Mma8451q, reg: u8) -> u8 {
    dev.start();
    dev.write(reg); // pointer byte
    dev.start(); // repeated START → read phase
    let v = dev.read();
    dev.stop();
    v
}

#[test]
fn who_am_i() {
    let mut dev = Mma8451q::new(0x1C);
    assert_eq!(read_reg(&mut dev, 0x0D), 0x1A);
}

#[test]
fn standby_freezes_output() {
    let mut dev = Mma8451q::new(0x1C);
    // Power-on: standby. Drive +1 g on X — output regs must stay 0 until ACTIVE.
    SimInput::set_input(&mut dev, "x", 1.0).unwrap();
    assert_eq!(read_reg(&mut dev, 0x01), 0x00, "standby must not convert");
    // CTRL_REG1.ACTIVE = 1
    dev.start();
    dev.write(0x2A);
    dev.write(0x01);
    dev.stop();
    assert_ne!(read_reg(&mut dev, 0x01), 0x00, "active must convert");
}

#[test]
fn one_g_x_encodes_14bit_left_justified() {
    let mut dev = Mma8451q::new(0x1C);
    dev.start();
    dev.write(0x2A);
    dev.write(0x01); // ACTIVE
    dev.stop();
    SimInput::set_input(&mut dev, "x", 1.0).unwrap();
    let msb = read_reg(&mut dev, 0x01);
    let lsb = read_reg(&mut dev, 0x02);
    let raw14 = ((msb as i16) << 8 | lsb as i16) >> 2; // signed 14-bit
                                                       // ±2g default → 4096 counts/g
    assert!((raw14 - 4096).abs() < 64, "raw14={raw14}");
}

#[test]
fn negative_g_is_twos_complement() {
    let mut dev = Mma8451q::new(0x1C);
    dev.start();
    dev.write(0x2A);
    dev.write(0x01);
    dev.stop();
    SimInput::set_input(&mut dev, "z", -1.0).unwrap();
    let msb = read_reg(&mut dev, 0x05);
    let lsb = read_reg(&mut dev, 0x06);
    let mut raw = ((msb as i16) << 8 | lsb as i16) >> 2;
    if raw & 0x2000 != 0 {
        raw |= !0x3FFF; // sign-extend 14 → 16
    }
    assert!((raw + 4096).abs() < 64, "raw={raw}");
}

#[test]
fn full_scale_select_changes_sensitivity() {
    let mut dev = Mma8451q::new(0x1C);
    // XYZ_DATA_CFG = ±8g (0b10) while still in standby
    dev.start();
    dev.write(0x0E);
    dev.write(0x02);
    dev.stop();
    dev.start();
    dev.write(0x2A);
    dev.write(0x01);
    dev.stop();
    SimInput::set_input(&mut dev, "x", 1.0).unwrap();
    let msb = read_reg(&mut dev, 0x01);
    let lsb = read_reg(&mut dev, 0x02);
    let raw14 = ((msb as i16) << 8 | lsb as i16) >> 2;
    // ±8g → 1024 counts/g
    assert!((raw14 - 1024).abs() < 32, "raw14={raw14}");
}

#[test]
fn noise_replays_identically_per_component() {
    let mut a = Mma8451q::new(0x1C).with_noise_sigma(0.01);
    let mut b = Mma8451q::new(0x1C).with_noise_sigma(0.01);
    for d in [&mut a, &mut b] {
        SimInput::set_component_id(d, "acc".into());
        d.start();
        d.write(0x2A);
        d.write(0x01);
        d.stop();
        SimInput::set_input(d, "y", 0.5).unwrap();
    }
    let ra: Vec<u8> = (0..6).map(|_| read_reg(&mut a, 0x03)).collect();
    let rb: Vec<u8> = (0..6).map(|_| read_reg(&mut b, 0x03)).collect();
    assert_eq!(ra, rb, "same seed+component must replay");
}
