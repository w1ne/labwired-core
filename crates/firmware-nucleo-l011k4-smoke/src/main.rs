// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO-L011K4: USART2 VCP output and D13/PB3 user LED transitions.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_CR: *mut u32 = RCC_BASE as *mut u32;
const RCC_CFGR: *mut u32 = (RCC_BASE + 0x0C) as *mut u32;
const RCC_IOPENR: *mut u32 = (RCC_BASE + 0x2C) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x38) as *mut u32;

const GPIOA_BASE: u32 = 0x5000_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_AFRH: *mut u32 = (GPIOA_BASE + 0x24) as *mut u32;

const GPIOB_BASE: u32 = 0x5000_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_BSRR: *mut u32 = (GPIOB_BASE + 0x18) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const LED_D13_PB3: u32 = 1 << 3;
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
        // Use HSI16 for a conventional 115200 baud USART2 configuration.
        write_volatile(RCC_CR, read_volatile(RCC_CR) | 1);
        while read_volatile(RCC_CR) & (1 << 2) == 0 {}
        write_volatile(RCC_CFGR, (read_volatile(RCC_CFGR) & !0b11) | 0b01);
        while read_volatile(RCC_CFGR) & (0b11 << 2) != 0b01 << 2 {}

        // Enable GPIOA/B and USART2 clocks.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | 0b11);
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));

        // USART2 VCP route is PA2 TX / PA15 RX, AF4 on STM32L011.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << 4) | (0b11 << 30))) | (0b10 << 4) | (0b10 << 30),
        );
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(GPIOA_AFRL, (afrl & !(0xF << 8)) | (4 << 8));
        let afrh = read_volatile(GPIOA_AFRH);
        write_volatile(GPIOA_AFRH, (afrh & !(0xF << 28)) | (4 << 28));

        // D13/PB3 is the board's active-high green user LED.
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !(0b11 << 6)) | (0b01 << 6));

        write_volatile(USART2_BRR, 139); // 16 MHz / 115200 baud, rounded.
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }

    puts(b"NUCLEO_L011K4_OK D13=PB3 USART2=PA2\n");

    // Exercise the on-board user LED as a real GPIO output path.
    for _ in 0..8 {
        unsafe { write_volatile(GPIOB_BSRR, LED_D13_PB3) };
        spin(64);
        unsafe { write_volatile(GPIOB_BSRR, LED_D13_PB3 << 16) };
        spin(64);
    }

    loop {
        core::hint::spin_loop();
    }
}
