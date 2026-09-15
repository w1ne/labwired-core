// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// SAMD21G18A smoke firmware for Arduino Nano 33 IoT: enable PM APBCMASK +
// GCLK for SERCOM5 (Serial1), print OK\n, toggle D13 (PA17). Bare-register
// driver — no HAL. Same binary targets silicon and the LabWired model.

#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

// PM APBCMASK @ 0x40000400 + 0x20; SERCOM5 is bit 7 (DS40001882).
const PM_APBCMASK: *mut u32 = 0x4000_0420 as *mut u32;
// GCLK CLKCTRL @ 0x40000C00 + 0x02 (16-bit).
const GCLK_CLKCTRL: *mut u16 = 0x4000_0C02 as *mut u16;
// SERCOM5 DATA @ 0x42001C00 + 0x28 (Nano 33 IoT Serial1).
const SERCOM5_DATA: *mut u32 = 0x4200_1C28 as *mut u32;
// PORTA DIRSET / OUTTGL (sam_port profile).
const PORTA_DIRSET: *mut u32 = 0x4100_4408 as *mut u32;
const PORTA_OUTTGL: *mut u32 = 0x4100_441C as *mut u32;

const LED: u32 = 1 << 17; // PA17 — D13 / LED_BUILTIN
                          // CLKCTRL: ID=25 (SERCOM5_CORE), GEN=0, CLKEN=1 → 0x4019
const GCLK_SERCOM5_CORE: u16 = 25 | (1 << 14);

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(PM_APBCMASK, 1 << 7);
        write_volatile(GCLK_CLKCTRL, GCLK_SERCOM5_CORE);
        write_volatile(PORTA_DIRSET, LED);
        for b in b"OK\n" {
            write_volatile(SERCOM5_DATA, *b as u32);
        }
        write_volatile(PORTA_OUTTGL, LED);
    }
    loop {}
}
