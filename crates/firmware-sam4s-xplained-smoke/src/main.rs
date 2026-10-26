// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM4S Xplained smoke firmware.
//!
//! The board's ATSAM4S16C is a Cortex-M4 with 1 MiB of flash at 0x00400000 and
//! 128 KiB of SRAM at 0x20000000. The console is UART0 (the board's virtual
//! COM port, PA9/PA10) and the user LED is PC10 (active high); the user button
//! is PA5 (active low, with the internal pull-up enabled here).
//!
//! The SAM4S PIO is the older parallel-I/O block: PIO_PER claims pins, PIO_OER
//! enables the output stage, PIO_SODR/PIO_CODR set and clear the output latch,
//! PIO_ODSR reads it back and PIO_PSR reads the pin levels. The PMC gates the
//! peripherals (PMC_PCER0) and reports the master clock ready in PMC_SR.
//!
//! Only the reset RC clock is used; the master clock is ready from reset, so
//! the firmware does not reprogram the PLL.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// PMC @ 0x400E0400: PCER0 @ 0x10, SR @ 0x68.
const PMC_PCER0: *mut u32 = 0x400E_0410 as *mut u32;
const PMC_SR: *const u32 = 0x400E_0468 as *const u32;
const PMC_SR_MCKRDY: u32 = 1 << 3;

// PIOA @ 0x400E0E00: PER @ 0x00, PSR @ 0x08, PUER @ 0x60.
const PIOA_PER: *mut u32 = 0x400E_0E00 as *mut u32;
const PIOA_PSR: *const u32 = 0x400E_0E08 as *const u32;
const PIOA_PUER: *mut u32 = 0x400E_0E60 as *mut u32;

// PIOC @ 0x400E1200: PER @ 0x00, OER @ 0x10, SODR @ 0x30, CODR @ 0x34,
// ODSR @ 0x38.
const PIOC_PER: *mut u32 = 0x400E_1200 as *mut u32;
const PIOC_OER: *mut u32 = 0x400E_1210 as *mut u32;
const PIOC_SODR: *mut u32 = 0x400E_1230 as *mut u32;
const PIOC_CODR: *mut u32 = 0x400E_1234 as *mut u32;
const PIOC_ODSR: *const u32 = 0x400E_1238 as *const u32;

// UART0 @ 0x400E0600: CR @ 0x00, MR @ 0x04, SR @ 0x14, THR @ 0x1C, BRGR @ 0x20.
const UART0_CR: *mut u32 = 0x400E_0600 as *mut u32;
const UART0_MR: *mut u32 = 0x400E_0604 as *mut u32;
const UART0_SR: *const u32 = 0x400E_0614 as *const u32;
const UART0_THR: *mut u32 = 0x400E_061C as *mut u32;
const UART0_BRGR: *mut u32 = 0x400E_0620 as *mut u32;

const UART_SR_TXRDY: u32 = 1 << 1;
const UART_CR_RXEN: u32 = 1 << 4;
const UART_CR_TXEN: u32 = 1 << 6;
/// 8 data bits, no parity, 1 stop bit (UART_MR CHRL=11, PAR=100).
const UART_MR_8N1: u32 = 0x8C0;

const LED_PIN: u32 = 1 << 10; // PC10
const BUTTON_PIN: u32 = 1 << 5; // PA5

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UART0_SR) & UART_SR_TXRDY == 0 {}
        write_volatile(UART0_THR, byte as u32);
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

fn odsr() -> u32 {
    unsafe { read_volatile(PIOC_ODSR) }
}

fn button_level() -> u32 {
    unsafe { (read_volatile(PIOA_PSR) >> 5) & 1 }
}

fn report(label: &str) {
    puts(label);
    puts(" odsr=");
    put_hex32(odsr());
    putc(b'\n');
}

#[entry]
fn main() -> ! {
    unsafe {
        // PMC: UART0 (id 8), PIOA (id 11) and PIOC (id 13).
        write_volatile(PMC_PCER0, (1 << 8) | (1 << 11) | (1 << 13));
        while read_volatile(PMC_SR) & PMC_SR_MCKRDY == 0 {}

        // UART0: 8N1, 115200 at 120 MHz (CD = 120e6 / 16 / 115200 = 65).
        write_volatile(UART0_MR, UART_MR_8N1);
        write_volatile(UART0_BRGR, 65);
        write_volatile(UART0_CR, UART_CR_RXEN | UART_CR_TXEN);

        // PA5 input with the internal pull-up (the board's button is
        // active-low; the pull-up is what makes a released pad read high).
        write_volatile(PIOA_PER, BUTTON_PIN);
        write_volatile(PIOA_PUER, BUTTON_PIN);

        // PC10 as a driven output.
        write_volatile(PIOC_PER, LED_PIN);
        write_volatile(PIOC_OER, LED_PIN);
    }

    puts("SAM4S XPLAINED SMOKE\n");
    puts("cpu=cortex-m4 uart=uart0\n");

    unsafe {
        write_volatile(PIOC_SODR, LED_PIN);
    }
    report("led=on");
    unsafe {
        write_volatile(PIOC_CODR, LED_PIN);
    }
    report("led=off");

    puts("btn=");
    put_dec32(button_level());
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        puts("hb=");
        put_dec32(heartbeat);
        puts(" odsr=");
        put_hex32(odsr());
        puts(" btn=");
        put_dec32(button_level());
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
