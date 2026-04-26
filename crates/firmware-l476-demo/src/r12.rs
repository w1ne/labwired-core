// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! Round 12: COMP + TSC + FMC register-state firmware.
//!
//! Exercises three peripherals added in this round:
//!   - COMP1/COMP2 — write a basic config (EN | INMSEL=011 | POLARITY)
//!     to COMP1_CSR, dump back. LOCK bit not set so subsequent writes
//!     are accepted.
//!   - TSC — TSCE | START with one group enabled in IOGCSR. Sim and
//!     silicon should both assert ISR.EOAF and clear START.
//!   - FMC — read reset values of BCR1 / SR / PCR.
//!
//! Output is a pure register-dump trace.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x48) as *mut u32;
const RCC_AHB2ENR: *mut u32 = (RCC_BASE + 0x4C) as *mut u32;
const RCC_AHB3ENR: *mut u32 = (RCC_BASE + 0x50) as *mut u32;
const RCC_APB1ENR1: *mut u32 = (RCC_BASE + 0x58) as *mut u32;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x60) as *mut u32;

const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4800_0020 as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const COMP_BASE: u32 = 0x4001_0200;
const COMP1_CSR: *mut u32 = COMP_BASE as *mut u32;
const COMP2_CSR: *mut u32 = (COMP_BASE + 0x04) as *mut u32;

const TSC_BASE: u32 = 0x4002_4000;
const TSC_CR: *mut u32 = TSC_BASE as *mut u32;
const TSC_ISR: *const u32 = (TSC_BASE + 0x0C) as *const u32;
const TSC_IOGCSR: *mut u32 = (TSC_BASE + 0x30) as *mut u32;

const FMC_BASE: u32 = 0xA000_0000;
const FMC_BCR1: *const u32 = FMC_BASE as *const u32;
const FMC_BTR1: *const u32 = (FMC_BASE + 0x04) as *const u32;
const FMC_PCR: *const u32 = (FMC_BASE + 0x80) as *const u32;
const FMC_SR: *const u32 = (FMC_BASE + 0x84) as *const u32;

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
    uart_puts(b"R12\r\n");

    // ---- COMP ----
    rmw(RCC_APB2ENR, 1 << 0); // SYSCFGEN — COMP shares with SYSCFG
    delay(50);
    unsafe {
        // EN | INMSEL=0b011 | POLARITY (bit 22)
        write_volatile(COMP1_CSR, 0x0040_0031);
    }
    uart_puts(b"COMP\r\n");
    dump(b"CSR1", COMP1_CSR);
    dump(b"CSR2", COMP2_CSR);

    // ---- TSC ----
    rmw(RCC_AHB1ENR, 1 << 16); // TSCEN
    delay(50);
    unsafe {
        // Enable group 1 + group 3 (GxE = 0b101).
        write_volatile(TSC_IOGCSR, 0x05);
        // TSCE | START.
        write_volatile(TSC_CR, 0x03);
    }
    delay(50);
    uart_puts(b"TSC\r\n");
    dump(b"CR  ", TSC_CR);
    dump(b"ISR ", TSC_ISR);
    dump(b"GCSR", TSC_IOGCSR);

    // ---- FMC ----
    rmw(RCC_AHB3ENR, 1 << 0); // FMCEN
    delay(50);
    uart_puts(b"FMC\r\n");
    dump(b"BCR1", FMC_BCR1);
    dump(b"BTR1", FMC_BTR1);
    dump(b"PCR ", FMC_PCR);
    dump(b"SR  ", FMC_SR);

    uart_puts(b"DONE\r\n");

    unsafe {
        while (read_volatile(USART2_ISR) & TC) == 0 {}
    }
    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}
