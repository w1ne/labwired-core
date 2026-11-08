// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Adafruit Feather M4 Express smoke: toggle the on-board D13 LED on PA23.

#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

// SAMD51 PORTA DIRSET and OUTTGL (DS60001507).
const PORTA_DIRSET: *mut u32 = 0x4100_8008 as *mut u32;
const PORTA_OUTTGL: *mut u32 = 0x4100_801C as *mut u32;
const LED_D13_PA23: u32 = 1 << 23;

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
        write_volatile(PORTA_DIRSET, LED_D13_PA23);
    }

    loop {
        unsafe {
            write_volatile(PORTA_OUTTGL, LED_D13_PA23);
        }
        spin(64);
    }
}
