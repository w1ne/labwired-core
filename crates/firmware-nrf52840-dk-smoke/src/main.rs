// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! nRF52840-DK: exercise physical LED1 (Zephyr alias led0), active-low on P0.13.

#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

const GPIO0_BASE: u32 = 0x5000_0000;
const GPIO0_OUTSET: *mut u32 = (GPIO0_BASE + 0x508) as *mut u32;
const GPIO0_OUTCLR: *mut u32 = (GPIO0_BASE + 0x50C) as *mut u32;
const GPIO0_DIRSET: *mut u32 = (GPIO0_BASE + 0x518) as *mut u32;
const GPIO0_PIN_CNF_13: *mut u32 = (GPIO0_BASE + 0x700 + 13 * 4) as *mut u32;
const BOARD_LED1_P0_13: u32 = 1 << 13;

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
        // Output, disconnected input buffer, default drive and no pull.
        write_volatile(GPIO0_PIN_CNF_13, 0b11);
        write_volatile(GPIO0_DIRSET, BOARD_LED1_P0_13);
        // The board LED is active-low: begin off, then produce repeated edges.
        write_volatile(GPIO0_OUTSET, BOARD_LED1_P0_13);
    }

    for _ in 0..8 {
        unsafe { write_volatile(GPIO0_OUTCLR, BOARD_LED1_P0_13) };
        spin(64);
        unsafe { write_volatile(GPIO0_OUTSET, BOARD_LED1_P0_13) };
        spin(64);
    }

    // Leave the physical LED on for the final-state oracle.
    unsafe { write_volatile(GPIO0_OUTCLR, BOARD_LED1_P0_13) };
    loop {
        core::hint::spin_loop();
    }
}
