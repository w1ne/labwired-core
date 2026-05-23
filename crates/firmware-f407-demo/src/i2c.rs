// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! F407 I²C survival-trace firmware.
//!
//! Round 2 of the F407 survival rotation. Drives the I²C1 state machine
//! without any actual slave on the bus and dumps register state at
//! each phase. Compares sim to silicon byte-for-byte via the same
//! dual-emit (USART2 + ARM semihosting) pattern as `smoke.rs`.
//!
//! Trace shape:
//!   I2C INIT
//!   CR1=...
//!   CR2=...
//!   CCR=...
//!   TRISE=...
//!   OAR1=...
//!   SR1=...
//!   SR2=...
//!   I2C START
//!   SR1=...
//!   I2C ADDR
//!   SR1=...
//!   SR2=...
//!   I2C STOP
//!   SR1=...
//!   SR2=...
//!   DONE
//!
//! With `nucleo-f407.yaml` having `external_devices: []` and no
//! hardware slave wired to the Discovery, neither sim nor silicon
//! should ACK the address. Sub-fixes likely surface in this round:
//! sim's `AddressPending` tick currently sets `ADDR` and `MSL`+`BUSY`
//! unconditionally even when `current_target` is `None`; real silicon
//! sets `SR1.AF` (bit 10) instead, leaves `ADDR` clear, leaves
//! `MSL`/`BUSY` clear. That's expected to be the round-2 divergence.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_3800;
const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x40) as *mut u32;

const GPIOA_BASE: u32 = 0x4002_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;

const GPIOB_BASE: u32 = 0x4002_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_OTYPER: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;
const GPIOB_PUPDR: *mut u32 = (GPIOB_BASE + 0x0C) as *mut u32;
const GPIOB_AFRL: *mut u32 = (GPIOB_BASE + 0x20) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR: *const u32 = USART2_BASE as *const u32;
const USART2_DR: *mut u32 = (USART2_BASE + 0x04) as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x08) as *mut u32;
const USART2_CR1: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;

const I2C1_BASE: u32 = 0x4000_5400;
const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_OAR1: *const u32 = (I2C1_BASE + 0x08) as *const u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *const u32 = (I2C1_BASE + 0x14) as *const u32;
const I2C1_SR2: *const u32 = (I2C1_BASE + 0x18) as *const u32;
const I2C1_CCR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_TRISE: *mut u32 = (I2C1_BASE + 0x20) as *mut u32;

const SR_TXE: u32 = 1 << 7;
const SR_TC: u32 = 1 << 6;

#[inline(always)]
fn rmw32(ptr: *mut u32, set: u32) {
    unsafe { write_volatile(ptr, read_volatile(ptr) | set) }
}

#[inline(always)]
fn rmw32_mask(ptr: *mut u32, clear: u32, set: u32) {
    unsafe {
        let v = read_volatile(ptr) & !clear;
        write_volatile(ptr, v | set);
    }
}

#[inline(always)]
fn semihost_writec(byte: u8) {
    let p = core::ptr::addr_of!(byte);
    #[cfg(target_arch = "arm")]
    unsafe {
        core::arch::asm!(
            "bkpt #0xAB",
            inout("r0") 0x03_u32 => _,
            in("r1") p,
            options(preserves_flags, nostack),
        );
    }
    #[cfg(not(target_arch = "arm"))]
    {
        let _ = (byte, p);
    }
}

fn uart_putc(c: u8) {
    unsafe {
        while (read_volatile(USART2_SR) & SR_TXE) == 0 {}
        write_volatile(USART2_DR, c as u32);
    }
    semihost_writec(c);
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

fn print_reg(label: &[u8], ptr: *const u32) {
    uart_puts(label);
    uart_putc(b'=');
    uart_put_hex32(unsafe { read_volatile(ptr) });
    uart_puts(b"\r\n");
}

fn usart2_init() {
    rmw32(RCC_AHB1ENR, 1 << 0); // GPIOAEN
    rmw32(RCC_APB1ENR, 1 << 17); // USART2EN
    rmw32_mask(GPIOA_MODER, 0x3 << 4, 0x2 << 4);
    rmw32_mask(GPIOA_MODER, 0x3 << 6, 0x2 << 6);
    rmw32_mask(GPIOA_AFRL, 0xF << 8, 0x7 << 8);
    rmw32_mask(GPIOA_AFRL, 0xF << 12, 0x7 << 12);
    unsafe {
        write_volatile(USART2_BRR, 139);
        write_volatile(USART2_CR1, (1 << 13) | (1 << 3));
    }
    for _ in 0..2_000 {
        unsafe { core::arch::asm!("nop") }
    }
}

fn i2c1_init() {
    // GPIOB clock + I²C1 clock.
    rmw32(RCC_AHB1ENR, 1 << 1); // GPIOBEN
    rmw32(RCC_APB1ENR, 1 << 21); // I2C1EN
                                 // PB6 SCL, PB7 SDA → AF4 (I²C1), open-drain, internal pull-up.
                                 // Discovery boards have no external pull-ups on PB6/PB7, so without
                                 // PUPDR=01 (pull-up) the lines float and the I²C peripheral latches
                                 // SR2.BUSY at boot — every subsequent START fails silently. Surfaced
                                 // by Round 2 capture: silicon showed SR2=0x02 (BUSY) from the very
                                 // first register read, SR1 stayed 0 across the whole transaction.
    rmw32_mask(GPIOB_MODER, 0x3 << 12, 0x2 << 12);
    rmw32_mask(GPIOB_MODER, 0x3 << 14, 0x2 << 14);
    rmw32(GPIOB_OTYPER, (1 << 6) | (1 << 7));
    rmw32_mask(GPIOB_PUPDR, 0x3 << 12, 0x1 << 12);
    rmw32_mask(GPIOB_PUPDR, 0x3 << 14, 0x1 << 14);
    rmw32_mask(GPIOB_AFRL, 0xF << 24, 0x4 << 24);
    rmw32_mask(GPIOB_AFRL, 0xF << 28, 0x4 << 28);
    // Reset I²C peripheral (CR1.SWRST then clear).
    unsafe {
        write_volatile(I2C1_CR1, 1 << 15);
        write_volatile(I2C1_CR1, 0);
    }
    // PCLK1 frequency = 16 MHz (HSI).
    unsafe { write_volatile(I2C1_CR2, 16) };
    // 100 kHz standard mode CCR = 80, TRISE = 17.
    unsafe { write_volatile(I2C1_CCR, 80) };
    unsafe { write_volatile(I2C1_TRISE, 17) };
    // Enable I²C peripheral.
    unsafe { write_volatile(I2C1_CR1, 1 << 0) };
}

/// Spin briefly so any pending state-machine transition can latch.
/// Both sim and silicon need a moment between issuing CR1.START / DR
/// writes and reading SR1.
fn brief_spin() {
    for _ in 0..2_000 {
        unsafe { core::arch::asm!("nop") }
    }
}

#[no_mangle]
pub extern "C" fn default_handler() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    usart2_init();
    i2c1_init();

    uart_puts(b"I2C INIT\r\n");
    print_reg(b"CR1", I2C1_CR1 as *const u32);
    print_reg(b"CR2", I2C1_CR2 as *const u32);
    print_reg(b"CCR", I2C1_CCR as *const u32);
    print_reg(b"TRISE", I2C1_TRISE as *const u32);
    print_reg(b"OAR1", I2C1_OAR1);
    print_reg(b"SR1", I2C1_SR1);
    print_reg(b"SR2", I2C1_SR2);

    // Issue START to a nonexistent slave (AHT20's nominal address; no
    // chip wired on this Discovery). Sim with `external_devices: []`
    // should also see no match.
    rmw32(I2C1_CR1, 1 << 8);
    brief_spin();
    uart_puts(b"I2C START\r\n");
    print_reg(b"SR1", I2C1_SR1);

    // Write address byte 0x70 = (0x38 << 1) | W. Triggers AddressPending
    // in sim or address phase on silicon.
    unsafe { write_volatile(I2C1_DR, 0x70) };
    brief_spin();
    uart_puts(b"I2C ADDR\r\n");
    print_reg(b"SR1", I2C1_SR1);
    print_reg(b"SR2", I2C1_SR2);

    // STOP.
    rmw32(I2C1_CR1, 1 << 9);
    brief_spin();
    uart_puts(b"I2C STOP\r\n");
    print_reg(b"SR1", I2C1_SR1);
    print_reg(b"SR2", I2C1_SR2);

    uart_puts(b"DONE\r\n");

    // Drain USART2 TC so the trailing newline isn't truncated.
    unsafe { while (read_volatile(USART2_SR) & SR_TC) == 0 {} }

    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}
