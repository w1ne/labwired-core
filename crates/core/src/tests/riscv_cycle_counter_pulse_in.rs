// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The RISC-V cycle counter must count CPU clocks at the simulated clock.
//!
//! Arduino-esp32 `pulseIn` on the ESP32-C3 times both its timeout and the
//! pulse in PCCR (CSR 0x7E2) ticks and converts with the CPU MHz. This runs
//! that algorithm against an HC-SR04 paced at 160 MHz and checks the distance.

use crate::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};
use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
use crate::{Bus, DebugControl, Machine};

const CPU_HZ: u64 = 160_000_000;
const CPU_MHZ: u64 = CPU_HZ / 1_000_000;
const GPIO_BASE: u64 = 0x4800_0000; // IDR @0x10, ODR @0x14
const RAM_BASE: u64 = 0x2000_0000;
const TRIG_BIT: u8 = 5;
const ECHO_BIT: u8 = 6;
const TIMEOUT_US: u32 = 30_000;

/// RV32I, a0=&ODR a1=TRIG a2=&IDR a3=ECHO a4=timeout cycles; a5 = pulse cycles
/// (0 on timeout), then spins at `END`.
const PULSE_IN: [u32; 23] = [
    0x00b5_2023, // sw a1,0(a0)          TRIG high
    0x0005_2023, // sw zero,0(a0)        TRIG low
    0x7e20_22f3, // csrr t0,0x7e2        start
    0x0006_2303, // 1: lw t1,0(a2)
    0x00d3_7333, // and t1,t1,a3
    0x0003_1a63, // bnez t1,2f
    0x7e20_23f3, // csrr t2,0x7e2
    0x4053_83b3, // sub t2,t2,t0
    0x0277_6463, // bltu a4,t2,4f        timeout
    0xfe9f_f06f, // j 1b
    0x7e20_2e73, // 2: csrr t3,0x7e2     pulse start
    0x0006_2303, // 3: lw t1,0(a2)
    0x00d3_7333, // and t1,t1,a3
    0x0003_0e63, // beqz t1,5f
    0x7e20_23f3, // csrr t2,0x7e2
    0x4053_83b3, // sub t2,t2,t0
    0x0077_6463, // bltu a4,t2,4f        timeout
    0xfe9f_f06f, // j 3b
    0x0000_0793, // 4: li a5,0
    0x00c0_006f, // j 6f
    0x7e20_2ef3, // 5: csrr t4,0x7e2
    0x41ce_87b3, // sub a5,t4,t3
    0x0000_006f, // 6: j 6b
];
const END: u32 = RAM_BASE as u32 + 4 * 22;

fn pulse_in_us(distance_cm: f64) -> u64 {
    let mut bus = crate::bus::SystemBus::new();
    let cpu = crate::system::riscv::configure_riscv(&mut bus);
    bus.add_peripheral(
        "gpio",
        GPIO_BASE,
        0x400,
        None,
        Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
    );
    let descriptor = labwired_config::DeviceDescriptor::embedded("hc-sr04")
        .unwrap()
        .unwrap();
    let mut device = DeclarativeGpioDevice::new(
        "U2".into(),
        &descriptor,
        vec![BoundPin {
            role: "TRIG".into(),
            addr: GPIO_BASE + 0x14,
            bit: TRIG_BIT,
        }],
        vec![BoundPin {
            role: "ECHO".into(),
            addr: GPIO_BASE + 0x10,
            bit: ECHO_BIT,
        }],
        CPU_HZ,
        crate::peripherals::components::declarative_i2c::owned_channels(&descriptor),
    )
    .unwrap();
    device.seed_input("distance", distance_cm);
    bus.gpio_devices.push(Box::new(device));

    let mut machine = Machine::new(cpu, bus);
    for (i, word) in PULSE_IN.iter().enumerate() {
        machine
            .bus
            .write_u32(RAM_BASE + 4 * i as u64, *word)
            .unwrap();
    }
    let x = &mut machine.cpu.x;
    x[10] = (GPIO_BASE + 0x14) as u32;
    x[11] = 1 << TRIG_BIT;
    x[12] = (GPIO_BASE + 0x10) as u32;
    x[13] = 1 << ECHO_BIT;
    x[14] = TIMEOUT_US * CPU_MHZ as u32;
    machine.cpu.pc = RAM_BASE as u32;

    // Timeout plus slack; the loop exits long before on a working counter.
    let budget = 2 * u64::from(TIMEOUT_US) * CPU_MHZ;
    while machine.cpu.pc != END && machine.total_cycles < budget {
        machine.run(Some(100_000)).unwrap();
    }
    assert_eq!(machine.cpu.pc, END, "pulseIn never returned");
    u64::from(machine.cpu.x[15]) / CPU_MHZ
}

#[test]
fn c3_pulse_in_reads_hc_sr04_distance() {
    for cm in [30u64, 150] {
        let us = pulse_in_us(cm as f64);
        // Same conversion as the sketch: 58 µs per cm, rounded.
        assert_eq!((us + 29) / 58, cm, "pulseIn = {us} µs at {cm} cm");
        assert!(us.abs_diff(cm * 58) <= 58, "pulseIn = {us} µs at {cm} cm");
    }
}
