// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WeAct STM32F411CEU6 Black Pill smoke: toggle the documented active-low PC13 LED.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const GPIOC_MODER: *mut u32 = 0x4002_0800 as *mut u32;
const GPIOC_BSRR: *mut u32 = 0x4002_0818 as *mut u32;
const LED_PC13: u32 = 1 << 13;

#[inline(never)]
fn pause() {
    for _ in 0..256 {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // GPIOC is clock-gated on AHB1. Set PC13 to general-purpose output.
        let ahb1enr = core::ptr::read_volatile(RCC_AHB1ENR);
        core::ptr::write_volatile(RCC_AHB1ENR, ahb1enr | (1 << 2));

        let moder = core::ptr::read_volatile(GPIOC_MODER);
        core::ptr::write_volatile(
            GPIOC_MODER,
            (moder & !(0b11 << (13 * 2))) | (0b01 << (13 * 2)),
        );

        loop {
            // PC13 low sinks current through the board's active-low LED.
            core::ptr::write_volatile(GPIOC_BSRR, LED_PC13 << 16);
            pause();
            core::ptr::write_volatile(GPIOC_BSRR, LED_PC13);
            pause();
        }
    }
}
