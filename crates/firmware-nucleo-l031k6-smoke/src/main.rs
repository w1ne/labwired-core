// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO-L031K6: exercise the active-high D13/PB3 user LED.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_CR: *mut u32 = RCC_BASE as *mut u32;
const RCC_CFGR: *mut u32 = (RCC_BASE + 0x0C) as *mut u32;
const RCC_IOPENR: *mut u32 = (RCC_BASE + 0x2C) as *mut u32;

const GPIOB_BASE: u32 = 0x5000_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_BSRR: *mut u32 = (GPIOB_BASE + 0x18) as *mut u32;

const LED_D13_PB3: u32 = 1 << 3;

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Select HSI16 from the STM32L0 reset clock before driving the LED.
        write_volatile(RCC_CR, read_volatile(RCC_CR) | 1);
        while read_volatile(RCC_CR) & (1 << 2) == 0 {}
        write_volatile(RCC_CFGR, (read_volatile(RCC_CFGR) & !0b11) | 0b01);
        while read_volatile(RCC_CFGR) & (0b11 << 2) != 0b01 << 2 {}

        // Enable GPIOB and configure PB3 (Arduino Nano D13 / LD3) as output.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | (1 << 1));
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !(0b11 << 6)) | (0b01 << 6));
    }

    // Generate repeatable transitions on the board's active-high user LED.
    for _ in 0..8 {
        unsafe { write_volatile(GPIOB_BSRR, LED_D13_PB3) };
        spin(64);
        unsafe { write_volatile(GPIOB_BSRR, LED_D13_PB3 << 16) };
        spin(64);
    }

    loop {
        core::hint::spin_loop();
    }
}
