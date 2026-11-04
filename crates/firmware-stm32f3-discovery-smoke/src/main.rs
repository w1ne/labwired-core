// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F3DISCOVERY smoke: toggle user LED LD3 on PE9.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const GPIOE_BASE: u32 = 0x4800_1000;
const GPIOE_MODER: *mut u32 = GPIOE_BASE as *mut u32;
const GPIOE_BSRR: *mut u32 = (GPIOE_BASE + 0x18) as *mut u32;
const GPIOEEN: u32 = 1 << 21;
const LED_LD3_PE9: u32 = 1 << 9;

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
        // Enable GPIOE (RM0316 RCC_AHBENR.GPIOEEN, bit 21).
        write_volatile(RCC_AHBENR, read_volatile(RCC_AHBENR) | GPIOEEN);

        // LD3 is connected to PE9; configure PE9 as a push-pull output.
        let moder = read_volatile(GPIOE_MODER);
        write_volatile(GPIOE_MODER, (moder & !(0b11 << 18)) | (0b01 << 18));

        loop {
            write_volatile(GPIOE_BSRR, LED_LD3_PE9);
            spin(64);
            write_volatile(GPIOE_BSRR, LED_LD3_PE9 << 16);
            spin(64);
        }
    }
}
