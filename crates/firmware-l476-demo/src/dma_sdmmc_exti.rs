// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! Round 11: DMA_CSELR + SDMMC + EXTI bank-2 register-state firmware.
//!
//! Exercises the registers added in this round: DMA1.CSELR (L4 channel
//! selection), SDMMC1 reset state + CMD-state-machine handshake, and
//! EXTI bank-2 IMR2/PR2 latching (via SWIER2).
//!
//! The output is a pure register-dump trace so sim and silicon can be
//! diffed byte-for-byte.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x48) as *mut u32;
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

const DMA1_BASE: u32 = 0x4002_0000;
const DMA1_CSELR: *mut u32 = (DMA1_BASE + 0xA8) as *mut u32;

const SDMMC1_BASE: u32 = 0x4001_2800;
const SDMMC1_POWER: *mut u32 = SDMMC1_BASE as *mut u32;
const SDMMC1_CLKCR: *mut u32 = (SDMMC1_BASE + 0x04) as *mut u32;
const SDMMC1_CMD: *mut u32 = (SDMMC1_BASE + 0x0C) as *mut u32;
const SDMMC1_RESPCMD: *const u32 = (SDMMC1_BASE + 0x10) as *const u32;
const SDMMC1_STA: *const u32 = (SDMMC1_BASE + 0x34) as *const u32;
const SDMMC1_ICR: *mut u32 = (SDMMC1_BASE + 0x38) as *mut u32;

const EXTI_BASE: u32 = 0x4001_0400;
const EXTI_IMR2: *mut u32 = (EXTI_BASE + 0x20) as *mut u32;
const EXTI_SWIER2: *mut u32 = (EXTI_BASE + 0x30) as *mut u32;
const EXTI_PR2: *const u32 = (EXTI_BASE + 0x34) as *const u32;

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
    uart_puts(b"R11\r\n");

    // ---- DMA1 + CSELR ----
    rmw(RCC_AHB1ENR, 1 << 0); // DMA1EN
    delay(50);
    unsafe {
        // Map ch1 -> request 4, ch7 -> request 5.
        write_volatile(DMA1_CSELR, (4 << 0) | (5 << 24));
    }
    uart_puts(b"DMA\r\n");
    dump(b"CSELR", DMA1_CSELR);

    // ---- SDMMC1: reset state + CMD-state-machine handshake ----
    rmw(RCC_APB2ENR, 1 << 10); // SDMMC1EN
    delay(50);
    unsafe {
        // Send a CMD with CPSMEN=1, WAITRESP=0 (no response).
        // Expect STA.CMDSENT (bit 7) to assert and RESPCMD = CMDINDEX.
        write_volatile(SDMMC1_CMD, 0x05 | (1 << 10));
    }
    uart_puts(b"SDMMC\r\n");
    dump(b"POWER ", SDMMC1_POWER);
    dump(b"CLKCR ", SDMMC1_CLKCR);
    dump(b"CMD   ", SDMMC1_CMD);
    dump(b"RSPCMD", SDMMC1_RESPCMD);
    dump(b"STA   ", SDMMC1_STA);

    // Clear CMDSENT via ICR.
    unsafe {
        write_volatile(SDMMC1_ICR, 1 << 7);
    }
    dump(b"STA-2 ", SDMMC1_STA);

    // ---- EXTI bank-2 ----
    unsafe {
        // Arm line 35 (LPUART1 wakeup), trigger via SWIER2.
        write_volatile(EXTI_IMR2, 1 << 3);
        write_volatile(EXTI_SWIER2, 1 << 3);
    }
    delay(50);
    uart_puts(b"EXTI2\r\n");
    dump(b"IMR2  ", EXTI_IMR2);
    dump(b"PR2   ", EXTI_PR2);

    uart_puts(b"DONE\r\n");

    unsafe {
        while (read_volatile(USART2_ISR) & TC) == 0 {}
    }
    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}
