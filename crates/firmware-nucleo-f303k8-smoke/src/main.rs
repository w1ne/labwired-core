// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO-F303K8 smoke: USART2 VCP output and LD3 on PB3.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_101C as *mut u32;

const GPIOA_BASE: u32 = 0x4800_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOB_BASE: u32 = 0x4800_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_BSRR: *mut u32 = (GPIOB_BASE + 0x18) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const LED_LD3_PB3: u32 = 1 << 3;
const USART_ISR_TXE: u32 = 1 << 7;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART2_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(USART2_TDR, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for &byte in text {
        putc(byte);
    }
}

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
        // Enable GPIOA/GPIOB and USART2 clocks (RM0316 RCC gate registers).
        write_volatile(
            RCC_AHBENR,
            read_volatile(RCC_AHBENR) | (1 << 17) | (1 << 18),
        );
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));

        // USART2_TX is AF7 on PA2, routed to the board's default ST-LINK VCP.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 4)) | (0b10 << 4));
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(GPIOA_AFRL, (afrl & !(0xF << 8)) | (7 << 8));

        // LD3 is active-high on PB3. Configure it as a push-pull output.
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !(0b11 << 6)) | (0b01 << 6));

        // Reset HSI is 8 MHz; BRR=69 is the nearest integer divisor for 115200.
        write_volatile(USART2_BRR, 69);
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }

    puts(b"NUCLEO_F303K8_OK\n");

    loop {
        unsafe {
            write_volatile(GPIOB_BSRR, LED_LD3_PB3);
        }
        spin(64);
        unsafe {
            write_volatile(GPIOB_BSRR, LED_LD3_PB3 << 16);
        }
        spin(64);
    }
}
