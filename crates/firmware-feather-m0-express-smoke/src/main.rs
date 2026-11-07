// LabWired - Adafruit Feather M0 Express smoke
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Enable the SAMD21 PORT clock and toggle Feather D13 / LED_BUILTIN (PA17).

#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

const PM_APBBMASK: *mut u32 = 0x4000_041C as *mut u32;
const PORTA_DIRSET: *mut u32 = 0x4100_4408 as *mut u32;
const PORTA_OUTCLR: *mut u32 = 0x4100_4414 as *mut u32;
const PORTA_OUTSET: *mut u32 = 0x4100_4418 as *mut u32;

const PORT_CLOCK_ENABLE: u32 = 1 << 3; // PM.APBBMASK.PORT_
const LED_D13_PA17: u32 = 1 << 17;

#[inline(never)]
fn delay() {
    for _ in 0..64 {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Exercise the PM -> PORT gate: a PORT write while disabled must not
        // configure PA17. Then enable the documented PORT APBB clock.
        write_volatile(PM_APBBMASK, 0);
        write_volatile(PORTA_DIRSET, LED_D13_PA17);
        write_volatile(PM_APBBMASK, PORT_CLOCK_ENABLE);

        write_volatile(PORTA_OUTCLR, LED_D13_PA17);
        write_volatile(PORTA_DIRSET, LED_D13_PA17);
    }

    loop {
        unsafe { write_volatile(PORTA_OUTSET, LED_D13_PA17) };
        delay();
        unsafe { write_volatile(PORTA_OUTCLR, LED_D13_PA17) };
        delay();
    }
}
