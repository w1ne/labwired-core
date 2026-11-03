// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Teensy 4.1 D13 LED smoke. This image deliberately exercises only CCM and
// GPIO2; it does not claim UART, FlexSPI/XIP, USB, Ethernet, or other blocks.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// CCM @ 0x400FC000 — CCGR0@0x68, GPIO2 CG15 in bits 31:30.
const CCM_CCGR0: *mut u32 = 0x400F_C068 as *mut u32;

// GPIO2 @ 0x401BC000 — GDIR@+0x04, DR_TOGGLE@+0x8C.
const GPIO2_GDIR: *mut u32 = 0x401B_C004 as *mut u32;
const GPIO2_DR_TOGGLE: *mut u32 = 0x401B_C08C as *mut u32;
const LED_D13: u32 = 1 << 3; // GPIO2_IO03 / Teensy 4.1 pin 13 LED.

#[entry]
fn main() -> ! {
    unsafe {
        // Enable the GPIO2 clock and configure the LED pad as an output.
        write_volatile(CCM_CCGR0, 0b11 << 30);
        let gdir = read_volatile(GPIO2_GDIR);
        write_volatile(GPIO2_GDIR, gdir | LED_D13);
    }

    loop {
        unsafe { write_volatile(GPIO2_DR_TOGGLE, LED_D13) };
        for _ in 0..64 {
            core::hint::spin_loop();
        }
    }
}
