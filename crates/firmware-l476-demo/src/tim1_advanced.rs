// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! TIM1 advanced-control bring-up firmware ("round 10").
//!
//! Exercises the registers that matter for centre-aligned / motor-control
//! PWM: BDTR (break + dead-time + MOE), RCR (repetition counter), CCMR1
//! (PWM mode 1 on OC1), CCER (CC1E + CC1NE complementary outputs).
//!
//! Output is a register-state dump — sim and silicon should agree
//! byte-for-byte after running through the canonical PWM-init sequence.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_AHB2ENR: *mut u32 = (RCC_BASE + 0x4C) as *mut u32;
const RCC_APB1ENR1: *mut u32 = (RCC_BASE + 0x58) as *mut u32;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x60) as *mut u32;

const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4800_0020 as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const TIM1_BASE: u32 = 0x4001_2C00;
const TIM1_CR1: *mut u32 = TIM1_BASE as *mut u32;
const TIM1_CR2: *mut u32 = (TIM1_BASE + 0x04) as *mut u32;
const TIM1_DIER: *mut u32 = (TIM1_BASE + 0x0C) as *mut u32;
const TIM1_SR: *mut u32 = (TIM1_BASE + 0x10) as *mut u32;
const TIM1_EGR: *mut u32 = (TIM1_BASE + 0x14) as *mut u32;
const TIM1_CCMR1: *mut u32 = (TIM1_BASE + 0x18) as *mut u32;
const TIM1_CCER: *mut u32 = (TIM1_BASE + 0x20) as *mut u32;
const TIM1_PSC: *mut u32 = (TIM1_BASE + 0x28) as *mut u32;
const TIM1_ARR: *mut u32 = (TIM1_BASE + 0x2C) as *mut u32;
const TIM1_RCR: *mut u32 = (TIM1_BASE + 0x30) as *mut u32;
const TIM1_CCR1: *mut u32 = (TIM1_BASE + 0x34) as *mut u32;
const TIM1_BDTR: *mut u32 = (TIM1_BASE + 0x44) as *mut u32;

const TXE: u32 = 1 << 7;
const TC: u32 = 1 << 6;

#[inline(always)]
fn delay(loops: u32) {
    for _ in 0..loops {
        unsafe { core::arch::asm!("nop") }
    }
}

#[inline(always)]
fn rmw(p: *mut u32, set: u32) {
    unsafe { write_volatile(p, read_volatile(p) | set); }
}

#[inline(always)]
fn rmw_mask(p: *mut u32, clear: u32, set: u32) {
    unsafe {
        let v = read_volatile(p) & !clear;
        write_volatile(p, v | set);
    }
}

fn uart_putc(c: u8) {
    unsafe {
        while (read_volatile(USART2_ISR) & TXE) == 0 {}
        write_volatile(USART2_TDR, c as u32);
    }
}

fn uart_puts(s: &[u8]) {
    for &b in s {
        uart_putc(b);
    }
}

fn uart_put_hex32(v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        uart_putc(HEX[((v >> (i * 4)) & 0xF) as usize]);
    }
}

fn dump(label: &[u8], reg: *const u32) {
    uart_puts(label);
    uart_putc(b'=');
    uart_put_hex32(unsafe { read_volatile(reg) });
    uart_puts(b"\r\n");
}

fn usart2_init() {
    rmw(RCC_AHB2ENR, 1 << 0);
    rmw(RCC_APB1ENR1, 1 << 17);
    rmw_mask(GPIOA_MODER, 0x3 << 4, 0x2 << 4);
    rmw_mask(GPIOA_AFRL, 0xF << 8, 0x7 << 8);
    unsafe {
        write_volatile(USART2_BRR, 35);
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3));
    }
    delay(5_000);
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    usart2_init();
    uart_puts(b"TIM1-ADV\r\n");

    // Enable TIM1 clock (APB2ENR bit 11).
    rmw(RCC_APB2ENR, 1 << 11);
    delay(100);

    // Configure PWM on channel 1 (canonical CubeMX-style sequence):
    //   1. PSC = 79 (80 MHz / 80 = 1 MHz timer clock)
    //   2. ARR = 999 (1 kHz PWM at 1 MHz timer clock)
    //   3. RCR = 5 (skip 5 update events between interrupts)
    //   4. CCR1 = 500 (50% duty cycle)
    //   5. CCMR1: OC1M = 0b110 (PWM mode 1), OC1PE = 1 (preload)
    //   6. CCER: CC1E + CC1NE (channel + complementary output enabled)
    //   7. BDTR: DTG=0x40 (dead-time), MOE (master output enable bit 15)
    //   8. EGR.UG to load preload values
    //   9. CR1.CEN to start
    unsafe {
        write_volatile(TIM1_PSC, 79);
        write_volatile(TIM1_ARR, 999);
        write_volatile(TIM1_RCR, 5);
        write_volatile(TIM1_CCR1, 500);
        // CCMR1: OC1M[2:0]=0b110 (bits 6:4), OC1PE (bit 3).
        write_volatile(TIM1_CCMR1, (0b110 << 4) | (1 << 3));
        // CCER: CC1E (bit 0) + CC1NE (bit 2).
        write_volatile(TIM1_CCER, (1 << 0) | (1 << 2));
        // BDTR: DTG=0x40 (bits 7:0), MOE (bit 15).
        write_volatile(TIM1_BDTR, (1 << 15) | 0x40);
        // EGR.UG to latch preload.
        write_volatile(TIM1_EGR, 1);
        // CR1.CEN.
        write_volatile(TIM1_CR1, 1);
    }

    delay(1000);

    // Dump the canonical PWM register state.
    uart_puts(b"CR1\r\n");
    dump(b"CR1 ", TIM1_CR1);
    uart_puts(b"PWM\r\n");
    dump(b"PSC ", TIM1_PSC);
    dump(b"ARR ", TIM1_ARR);
    dump(b"RCR ", TIM1_RCR);
    dump(b"CCR1", TIM1_CCR1);
    dump(b"CCMR", TIM1_CCMR1);
    dump(b"CCER", TIM1_CCER);
    dump(b"BDTR", TIM1_BDTR);

    uart_puts(b"DONE\r\n");

    unsafe {
        while (read_volatile(USART2_ISR) & TC) == 0 {}
    }
    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}
