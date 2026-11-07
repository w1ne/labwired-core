// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F0DISCOVERY smoke: toggles green user LED LD3 on PC9.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const GPIOC_BASE: u32 = 0x4800_0800;
const GPIOC_MODER: *mut u32 = GPIOC_BASE as *mut u32;
const GPIOC_BSRR: *mut u32 = (GPIOC_BASE + 0x18) as *mut u32;
const LED_LD3: u32 = 1 << 9;

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
        // RM0091 RCC_AHBENR.IOPCEN (bit 19) gates GPIOC.
        write_volatile(RCC_AHBENR, read_volatile(RCC_AHBENR) | (1 << 19));

        // LD3 is active-high and wired to PC9. Set its MODER field to output.
        let moder = read_volatile(GPIOC_MODER);
        write_volatile(GPIOC_MODER, (moder & !(0b11 << 18)) | (0b01 << 18));
    }

    // Four complete on/off cycles provide eight observable PC9 transitions.
    for _ in 0..4 {
        unsafe { write_volatile(GPIOC_BSRR, LED_LD3) };
        spin(32);
        unsafe { write_volatile(GPIOC_BSRR, LED_LD3 << 16) };
        spin(32);
    }

    loop {
        core::hint::spin_loop();
    }
}
