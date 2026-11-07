// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO-C051C8 smoke: enable GPIOA and toggle LD4 on PA5.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_IOPENR: *mut u32 = 0x4002_1034 as *mut u32;
const GPIOA_BASE: u32 = 0x5000_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const LED_LD4_PA5: u32 = 1 << 5;

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
        // STM32C0 RCC_IOPENR@0x34: GPIOAEN is bit 0.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | 1);

        // The MB1717 board's default SB26 route connects PA5 to active-high LD4.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));
    }

    for _ in 0..8 {
        unsafe { write_volatile(GPIOA_BSRR, LED_LD4_PA5) };
        spin(16);
        unsafe { write_volatile(GPIOA_BSRR, LED_LD4_PA5 << 16) };
        spin(16);
    }

    loop {
        core::hint::spin_loop();
    }
}
