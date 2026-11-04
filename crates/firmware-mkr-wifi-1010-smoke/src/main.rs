// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! Arduino MKR WiFi 1010 smoke: enable SAMD21 PORT on APBB, then toggle D6 /
//! LED_BUILTIN (PA20). The board's NINA Wi-Fi/BLE and ECC508 are not exercised.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const PM_APBBMASK: *mut u32 = 0x4000_041C as *mut u32;
const PORTA_DIRSET: *mut u32 = 0x4100_4408 as *mut u32;
const PORTA_OUT: *const u32 = 0x4100_4410 as *const u32;
const PORTA_OUTCLR: *mut u32 = 0x4100_4414 as *mut u32;
const PORTA_OUTSET: *mut u32 = 0x4100_4418 as *mut u32;

const PORT_CLOCK_ENABLE: u32 = 1 << 3; // PM.APBBMASK.PORT_
const LED_D6: u32 = 1 << 20; // MKR WiFi 1010 D6 / LED_BUILTIN = PA20

#[entry]
fn main() -> ! {
    unsafe {
        // Start with PORT gated off, then enable its documented APBB clock.
        // This exercises the PM->PORT gate in the profile before the LED path.
        write_volatile(PM_APBBMASK, 0);
        write_volatile(PORTA_DIRSET, LED_D6);
        write_volatile(PM_APBBMASK, PORT_CLOCK_ENABLE);

        // PA20 drives Q1, which sinks current through the onboard yellow LED.
        write_volatile(PORTA_OUTCLR, LED_D6);
        write_volatile(PORTA_DIRSET, LED_D6);
    }

    loop {
        unsafe { write_volatile(PORTA_OUTSET, LED_D6) };
        delay();
        unsafe { write_volatile(PORTA_OUTCLR, LED_D6) };
        delay();
    }
}

#[inline(never)]
fn delay() {
    for _ in 0..64 {
        // Keep each interval observable to the interpreter and to the GPIO
        // trace instead of allowing an optimized empty loop to disappear.
        unsafe {
            let _ = read_volatile(PORTA_OUT);
        }
    }
}
