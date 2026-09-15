// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// SAMD51J19A smoke firmware for Adafruit Metro M4: enable MCLK APBBMASK +
// GCLK PCHCTRL for SERCOM3 (Serial1), print OK\n, toggle D13 (PA16).
// Bare-register driver — no HAL. Same binary targets silicon and the model.

#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

// MCLK APBBMASK @ 0x40000800 + 0x18; SERCOM3 is bit 10 (DS60001507).
const MCLK_APBBMASK: *mut u32 = 0x4000_0818 as *mut u32;
// GCLK PCHCTRL[24] @ 0x40001C00 + 0x80 + 4*24 (SERCOM3_CORE).
const GCLK_PCHCTRL24: *mut u32 = 0x4000_1CE0 as *mut u32;
// SERCOM3 DATA @ 0x41014000 + 0x28 (Metro M4 Serial1).
const SERCOM3_DATA: *mut u32 = 0x4101_4028 as *mut u32;
// PORTA DIRSET / OUTTGL (sam_port profile @ 0x41008000).
const PORTA_DIRSET: *mut u32 = 0x4100_8008 as *mut u32;
const PORTA_OUTTGL: *mut u32 = 0x4100_801C as *mut u32;

const LED: u32 = 1 << 16; // PA16 — D13 / LED_BUILTIN
                          // PCHCTRL: GEN=0 | CHEN=1 → bit 6
const GCLK_PCHCTRL_CHEN: u32 = 1 << 6;

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(MCLK_APBBMASK, 1 << 10);
        write_volatile(GCLK_PCHCTRL24, GCLK_PCHCTRL_CHEN);
        write_volatile(PORTA_DIRSET, LED);
        for b in b"OK\n" {
            write_volatile(SERCOM3_DATA, *b as u32);
        }
        write_volatile(PORTA_OUTTGL, LED);
    }
    loop {}
}
