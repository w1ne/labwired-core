// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO-C031C6 smoke: enable GPIOA and toggle the on-board LD4 on PA5.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_IOPENR: *mut u32 = 0x4002_1034 as *mut u32;
const GPIOA_BASE: u32 = 0x5000_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const LED_LD4: u32 = 1 << 5;

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
        // RM0490 §6.4.12: GPIOAEN is bit 0 in RCC_IOPENR@0x34.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | 1);

        // LD4 is on PA5 (UM2953 Rev 2; active high).
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));
    }

    // Produce observable PA5 transitions, then idle for the smoke runner.
    for _ in 0..8 {
        unsafe { write_volatile(GPIOA_BSRR, LED_LD4) };
        spin(16);
        unsafe { write_volatile(GPIOA_BSRR, LED_LD4 << 16) };
        spin(16);
    }

    loop {
        core::hint::spin_loop();
    }
}
