// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// TIMER-driven blinky on the XIAO nRF52840 Sense pinout (LED_RED on
// P0.26). Used by the labwired-core integration test to prove that the
// simulator's TIMER + GPIO + Cortex-M execution stack is good enough
// for real firmware: the test loads this as an ELF, steps the CPU,
// and observes GPIO0 OUT bit 26 toggling.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const GPIO0_BASE: usize = 0x5000_0000;
const GPIO0_OUT: *mut u32 = (GPIO0_BASE + 0x504) as *mut u32;
const GPIO0_DIRSET: *mut u32 = (GPIO0_BASE + 0x518) as *mut u32;

const TIMER0_BASE: usize = 0x4000_8000;
const TIMER0_TASKS_START: *mut u32 = TIMER0_BASE as *mut u32;
const TIMER0_TASKS_CLEAR: *mut u32 = (TIMER0_BASE + 0x00C) as *mut u32;
const TIMER0_EVENTS_COMPARE0: *mut u32 = (TIMER0_BASE + 0x140) as *mut u32;
const TIMER0_SHORTS: *mut u32 = (TIMER0_BASE + 0x200) as *mut u32;
const TIMER0_BITMODE: *mut u32 = (TIMER0_BASE + 0x508) as *mut u32;
const TIMER0_PRESCALER: *mut u32 = (TIMER0_BASE + 0x510) as *mut u32;
const TIMER0_CC0: *mut u32 = (TIMER0_BASE + 0x540) as *mut u32;

const LED_RED_BIT: u32 = 1 << 26;

#[entry]
fn main() -> ! {
    unsafe {
        // Configure LED pin as output.
        write_volatile(GPIO0_DIRSET, LED_RED_BIT);

        // TIMER0: 32-bit, no prescaler, CC[0]=8, COMPARE_CLEAR short.
        write_volatile(TIMER0_BITMODE, 3);
        write_volatile(TIMER0_PRESCALER, 0);
        write_volatile(TIMER0_CC0, 8);
        write_volatile(TIMER0_SHORTS, 1);
        write_volatile(TIMER0_TASKS_CLEAR, 1);
        write_volatile(TIMER0_TASKS_START, 1);
    }

    let mut led_on = false;

    loop {
        // Busy-wait for TIMER0 EVENTS_COMPARE[0] to fire.
        while unsafe { read_volatile(TIMER0_EVENTS_COMPARE0) } == 0 {}
        unsafe { write_volatile(TIMER0_EVENTS_COMPARE0, 0) };

        led_on = !led_on;
        let new_out = if led_on { LED_RED_BIT } else { 0 };
        unsafe { write_volatile(GPIO0_OUT, new_out) };
    }
}
