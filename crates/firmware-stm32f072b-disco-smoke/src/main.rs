// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F072B-DISCO smoke firmware.
//!
//! The board's STM32F072RBT6 is a Cortex-M0 with 128 KiB of flash at
//! 0x08000000 and 16 KiB of SRAM at 0x20000000. The console is USART1
//! (PA9/PA10, AF1) — the ST-LINK/V2-1 virtual COM port. The four user LEDs
//! are PC6 (LD3 red), PC8 (LD4 yellow), PC9 (LD5 green) and PC7 (LD6 blue),
//! all active high; the user button B1 pulls PA0 low.
//!
//! The firmware enables the F0 clock gates (AHBENR for the GPIOs, APB2ENR for
//! USART1), configures the pins, walks the LEDs and prints the GPIOC ODR
//! readback, reads the button level and then heartbeats.
//!
//! Only the reset HSI (8 MHz) is used: BRR = 8 MHz / 115200 = 69 (0x45) for
//! the USART v2 block at the F0's peripheral clock.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC (RM0360 §6): AHBENR @ 0x14, APB2ENR @ 0x18.
const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;

// GPIOA @ 0x48000000 (AHB): MODER @ 0x00, PUPDR @ 0x0C, IDR @ 0x10, AFRH @ 0x24.
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_PUPDR: *mut u32 = 0x4800_000C as *mut u32;
const GPIOA_IDR: *const u32 = 0x4800_0010 as *const u32;
const GPIOA_AFRH: *mut u32 = 0x4800_0024 as *mut u32;

// GPIOC @ 0x48000800: MODER @ 0x00, ODR @ 0x14, BSRR @ 0x18.
const GPIOC_MODER: *mut u32 = 0x4800_0800 as *mut u32;
const GPIOC_ODR: *const u32 = 0x4800_0814 as *const u32;
const GPIOC_BSRR: *mut u32 = 0x4800_0818 as *mut u32;

// USART1 @ 0x40013800: CR1 @ 0x00, BRR @ 0x0C, ISR @ 0x1C, TDR @ 0x28.
const USART1_CR1: *mut u32 = 0x4001_3800 as *mut u32;
const USART1_BRR: *mut u32 = 0x4001_380C as *mut u32;
const USART1_ISR: *const u32 = 0x4001_381C as *const u32;
const USART1_TDR: *mut u32 = 0x4001_3828 as *mut u32;

const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_RE: u32 = 1 << 2;
const USART_CR1_TE: u32 = 1 << 3;

const LED_LD3: u32 = 1 << 6; // PC6, red
const LED_LD4: u32 = 1 << 8; // PC8, yellow
const LED_LD5: u32 = 1 << 9; // PC9, green
const LED_LD6: u32 = 1 << 7; // PC7, blue

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART1_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(USART1_TDR, byte as u32);
    }
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

fn put_hex32(value: u32) {
    for shift in (0..8).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

fn put_dec32(value: u32) {
    let mut digits = [0u8; 10];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
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

fn led_on(mask: u32) {
    unsafe {
        write_volatile(GPIOC_BSRR, mask);
    }
}

fn report(label: &str) {
    puts(label);
    puts(" odr=");
    put_hex32(unsafe { read_volatile(GPIOC_ODR) });
    putc(b'\n');
}

fn button_level() -> u32 {
    unsafe { read_volatile(GPIOA_IDR) & 1 }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Gate GPIOA/GPIOC (AHBENR bits 17/19) and USART1 (APB2ENR bit 14).
        write_volatile(
            RCC_AHBENR,
            read_volatile(RCC_AHBENR) | (1 << 17) | (1 << 19),
        );
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | (1 << 14));

        // PA9/PA10 as AF1 (USART1): MODER 10, AFRH nibble 1 per pin.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !(0b1111 << 18)) | (0b10 << 18) | (0b10 << 20),
        );
        write_volatile(
            GPIOA_AFRH,
            (read_volatile(GPIOA_AFRH) & !(0xFF << 4)) | (0x11 << 4),
        );
        // PA0 input with the internal pull-up (B1 pulls it low when pressed).
        write_volatile(GPIOA_PUPDR, (read_volatile(GPIOA_PUPDR) & !0b11) | 0b01);

        // PC6..PC9 as outputs.
        write_volatile(
            GPIOC_MODER,
            (read_volatile(GPIOC_MODER) & !(0xFF << 12)) | (0x55 << 12),
        );

        // USART1: 8 MHz HSI / 115200 = 69 = 0x45.
        write_volatile(USART1_BRR, 0x45);
        write_volatile(USART1_CR1, USART_CR1_UE | USART_CR1_TE | USART_CR1_RE);
    }

    puts("STM32F072B-DISCO SMOKE\n");
    puts("cpu=cortex-m0 uart=usart1\n");

    led_on(LED_LD3);
    report("led=ld3");
    led_on(LED_LD4);
    report("led=ld4");
    led_on(LED_LD5);
    report("led=ld5");
    led_on(LED_LD6);
    report("led=ld6");

    puts("btn=");
    put_dec32(button_level());
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        puts("hb=");
        put_dec32(heartbeat);
        puts(" odr=");
        put_hex32(unsafe { read_volatile(GPIOC_ODR) });
        puts(" btn=");
        put_dec32(button_level());
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
