// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! L4 secondary-peripheral exercise firmware ("round 8").
//!
//! Touches every peripheral added in the second-wave L4 expansion
//! (LPUART1, LPTIM1, EXTI L4 layout, QUADSPI, SAI1, USB OTG FS, bxCAN1)
//! and dumps a register snapshot over USART2 so the simulator and real
//! silicon can be diffed byte-for-byte.
//!
//! This trace is locked as `nucleo_l476rg_l4periphs2_survival`. The first
//! lock is the simulator's own output (audited from RM0351 reset values);
//! a hardware capture from the bench will close the loop and convert the
//! test into a true silicon-parity gate.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

// ---------- Reset/clock plumbing (USART2 + peripheral clocks) -------------

const RCC_BASE:        u32 = 0x4002_1000;
const RCC_AHB2ENR:     *mut u32 = (RCC_BASE + 0x4C) as *mut u32;
const RCC_AHB3ENR:     *mut u32 = (RCC_BASE + 0x50) as *mut u32;
const RCC_APB1ENR1:    *mut u32 = (RCC_BASE + 0x58) as *mut u32;
const RCC_APB1ENR2:    *mut u32 = (RCC_BASE + 0x5C) as *mut u32;
const RCC_APB2ENR:     *mut u32 = (RCC_BASE + 0x60) as *mut u32;

const GPIOA_MODER:     *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL:      *mut u32 = 0x4800_0020 as *mut u32;

const USART2_BASE:     u32 = 0x4000_4400;
const USART2_CR1:      *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR:      *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR:      *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR:      *mut u32 = (USART2_BASE + 0x28) as *mut u32;

// ---------- New peripheral bases (RM0351) ---------------------------------

const LPUART1_BASE:    u32 = 0x4000_8000;
const LPTIM1_BASE:     u32 = 0x4000_7C00;
const EXTI_BASE:       u32 = 0x4001_0400;
const QUADSPI_BASE:    u32 = 0xA000_1000;
const SAI1_BASE:       u32 = 0x4001_5400;
const OTG_BASE:        u32 = 0x5000_0000;
const CAN1_BASE:       u32 = 0x4000_6400;

const TXE: u32 = 1 << 7;
const TC:  u32 = 1 << 6;

#[inline(always)]
fn delay(loops: u32) {
    for _ in 0..loops {
        unsafe { core::arch::asm!("nop") }
    }
}

#[inline(always)]
fn rmw32(ptr: *mut u32, set: u32) {
    unsafe { write_volatile(ptr, read_volatile(ptr) | set); }
}

#[inline(always)]
fn rmw32_mask(ptr: *mut u32, clear: u32, set: u32) {
    unsafe {
        let v = read_volatile(ptr) & !clear;
        write_volatile(ptr, v | set);
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

fn dump(label: &[u8], addr: u32) {
    uart_puts(label);
    uart_putc(b'=');
    uart_put_hex32(unsafe { read_volatile(addr as *const u32) });
    uart_puts(b"\r\n");
}

fn usart2_init() {
    rmw32(RCC_AHB2ENR, 1 << 0);   // GPIOAEN
    rmw32(RCC_APB1ENR1, 1 << 17); // USART2EN
    rmw32_mask(GPIOA_MODER, 0x3 << 4, 0x2 << 4);
    rmw32_mask(GPIOA_AFRL,  0xF << 8, 0x7 << 8);
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
    uart_puts(b"L4-PERIPHS2\r\n");

    // ---- LPUART1: enable clock, dump reset register state ---------------
    rmw32(RCC_APB1ENR2, 1 << 0); // LPUART1EN (APB1ENR2 bit 0)
    delay(100);
    uart_puts(b"LPUART1\r\n");
    dump(b"CR1", LPUART1_BASE);
    dump(b"ISR", LPUART1_BASE + 0x1C);
    dump(b"BRR", LPUART1_BASE + 0x0C);

    // ---- LPTIM1: enable clock, write ARR/CMP, check ARROK/CMPOK --------
    rmw32(RCC_APB1ENR1, 1 << 31); // LPTIM1EN (APB1ENR1 bit 31)
    delay(100);
    unsafe {
        // CFGR: leave default (internal clock, no prescaler).
        write_volatile((LPTIM1_BASE + 0x0C) as *mut u32, 0);
        // CR.ENABLE
        write_volatile((LPTIM1_BASE + 0x10) as *mut u32, 1);
        // ARR <- 0x1000
        write_volatile((LPTIM1_BASE + 0x18) as *mut u32, 0x1000);
        // CMP <- 0x0800
        write_volatile((LPTIM1_BASE + 0x14) as *mut u32, 0x0800);
    }
    delay(50);
    uart_puts(b"LPTIM1\r\n");
    dump(b"ISR", LPTIM1_BASE);
    dump(b"ARR", LPTIM1_BASE + 0x18);
    dump(b"CMP", LPTIM1_BASE + 0x14);

    // ---- EXTI: program line 22 (RTC alarm), trigger via SWIER ----------
    unsafe {
        write_volatile((EXTI_BASE + 0x00) as *mut u32, 1 << 22); // IMR1
        write_volatile((EXTI_BASE + 0x10) as *mut u32, 1 << 22); // SWIER1
    }
    delay(50);
    uart_puts(b"EXTI\r\n");
    dump(b"IMR1", EXTI_BASE);
    dump(b"PR1 ", EXTI_BASE + 0x14);
    // L4-only bank-2 access. Line 35 (USB FS wakeup).
    unsafe {
        write_volatile((EXTI_BASE + 0x20) as *mut u32, 1 << (35 - 32)); // IMR2
    }
    dump(b"IMR2", EXTI_BASE + 0x20);

    // ---- QUADSPI: enable clock, peek reset state ------------------------
    rmw32(RCC_AHB3ENR, 1 << 8); // QSPIEN
    delay(100);
    uart_puts(b"QUADSPI\r\n");
    dump(b"CR ", QUADSPI_BASE);
    dump(b"DCR", QUADSPI_BASE + 0x04);
    dump(b"SR ", QUADSPI_BASE + 0x08);

    // ---- SAI1: enable clock, peek reset state ---------------------------
    rmw32(RCC_APB2ENR, 1 << 21); // SAI1EN
    delay(100);
    uart_puts(b"SAI1\r\n");
    dump(b"GCR ", SAI1_BASE);
    dump(b"ACR1", SAI1_BASE + 0x04);
    dump(b"BCR1", SAI1_BASE + 0x24);

    // ---- USB OTG FS: enable clock, peek core regs -----------------------
    rmw32(RCC_AHB2ENR, 1 << 12); // OTGFSEN
    delay(100);
    uart_puts(b"OTG\r\n");
    dump(b"GUSBCFG", OTG_BASE + 0x0C);
    dump(b"GRSTCTL", OTG_BASE + 0x10);
    dump(b"GINTSTS", OTG_BASE + 0x14);

    // ---- CAN1: enable clock, INRQ handshake -----------------------------
    rmw32(RCC_APB1ENR1, 1 << 25); // CAN1EN
    delay(100);
    unsafe {
        // Set INRQ.
        write_volatile(CAN1_BASE as *mut u32, 1);
    }
    // Wait for INAK (MSR bit 0).
    let mut waited = 0u32;
    while waited < 100 {
        if unsafe { read_volatile((CAN1_BASE + 0x04) as *const u32) } & 1 != 0 {
            break;
        }
        waited += 1;
    }
    uart_puts(b"CAN1\r\n");
    dump(b"MCR", CAN1_BASE);
    dump(b"MSR", CAN1_BASE + 0x04);
    dump(b"TSR", CAN1_BASE + 0x08);

    uart_puts(b"DONE\r\n");

    unsafe {
        while (read_volatile(USART2_ISR) & TC) == 0 {}
    }
    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}
