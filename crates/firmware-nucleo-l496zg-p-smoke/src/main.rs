#![no_std]
#![no_main]

//! NUCLEO-L496ZG-P: boot the STM32L496ZGT6P and toggle onboard LD3 on PB14.
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4800_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4800_0418 as *mut u32;
const LED_PB14: u32 = 1 << 14;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB, configure PB14 as an output, and exercise LD3.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | (1 << 1));
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !(0b11 << 28)) | (0b01 << 28));

        for _ in 0..8 {
            write_volatile(GPIOB_BSRR, LED_PB14);
            for _ in 0..16 {
                cortex_m::asm::nop();
            }
            write_volatile(GPIOB_BSRR, LED_PB14 << 16);
            for _ in 0..16 {
                cortex_m::asm::nop();
            }
        }
    }

    loop {
        cortex_m::asm::nop();
    }
}
