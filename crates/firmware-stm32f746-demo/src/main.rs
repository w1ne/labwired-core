// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// STM32F746 Discovery smoke: ungating RCC clocks, print OK\n via USART1 TDR,
// toggle PI1 (LD1) via BSRR. Bare-register — no HAL.
// Soft-float thumbv7em-none-eabi. Same binary targets the LabWired model.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC @ 0x40023800 — AHB1ENR@0x30, APB2ENR@0x44 (RM0385 / F4-compatible).
const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_3844 as *mut u32;

// USART1 @ 0x40011000 — CR1@+0x00, ISR@+0x1C, TDR@+0x28 (stm32v2).
const USART1_CR1: *mut u32 = 0x4001_1000 as *mut u32;
const USART1_ISR: *mut u32 = 0x4001_101C as *mut u32;
const USART1_TDR: *mut u32 = 0x4001_1028 as *mut u32;

const CR1_UE: u32 = 1 << 0;
const CR1_TE: u32 = 1 << 3;
const ISR_TXE: u32 = 1 << 7;

// GPIOI @ 0x40022000 — MODER@+0x00, BSRR@+0x18 (stm32v2).
const GPIOI_MODER: *mut u32 = 0x4002_2000 as *mut u32;
const GPIOI_BSRR: *mut u32 = 0x4002_2018 as *mut u32;

const LED: u32 = 1 << 1; // PI1 — Discovery green LD1

#[entry]
fn main() -> ! {
    unsafe {
        // GPIOIEN (AHB1ENR bit 8) + USART1EN (APB2ENR bit 4).
        write_volatile(RCC_AHB1ENR, read_volatile(RCC_AHB1ENR) | (1 << 8));
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | (1 << 4));

        // PI1 output (MODER bits 3:2 = 01).
        let moder = read_volatile(GPIOI_MODER);
        write_volatile(GPIOI_MODER, (moder & !(0b11 << 2)) | (0b01 << 2));

        // UE | TE — enable USART1 transmitter.
        write_volatile(USART1_CR1, CR1_UE | CR1_TE);

        for b in b"OK\n" {
            while read_volatile(USART1_ISR) & ISR_TXE == 0 {}
            write_volatile(USART1_TDR, *b as u32);
        }

        // BSRR set PI1 (active-high LED on).
        write_volatile(GPIOI_BSRR, LED);
    }
    loop {}
}
