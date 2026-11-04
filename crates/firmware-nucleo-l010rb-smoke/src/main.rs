#![no_std]
#![no_main]

//! NUCLEO-L010RB: boot the Cortex-M0+ and toggle the onboard LD2/D13 LED.
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_IOPENR: *mut u32 = 0x4002_102C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x5000_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x5000_0018 as *mut u32;
const LED_PA5: u32 = 1 << 5;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA on the L0 I/O bus, then configure PA5 as an output.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | 1);
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));

        // LD2 is active-high on PA5. Repeated set/reset writes exercise the
        // board-connected GPIO path visible to the simulator's edge capture.
        for _ in 0..8 {
            write_volatile(GPIOA_BSRR, LED_PA5);
            for _ in 0..16 {
                cortex_m::asm::nop();
            }
            write_volatile(GPIOA_BSRR, LED_PA5 << 16);
            for _ in 0..16 {
                cortex_m::asm::nop();
            }
        }
    }

    loop {
        cortex_m::asm::nop();
    }
}
