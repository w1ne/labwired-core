// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! LiteX Minerva SoC smoke firmware.
//!
//! Boots from the SoC's integrated SRAM (fast boot: the ELF is loaded
//! directly, the LiteX BIOS and the boot-flash load are skipped) and prints
//! over the LiteX UART at 0xE0001800:
//!
//! * a banner and the modelled-surface summary,
//! * an add/shift-only arithmetic result (the Minerva core is RV32I — the
//!   firmware is built for `riscv32i-unknown-none-elf`, so the toolchain
//!   refuses multiply/divide at compile time),
//! * a readback of the timer CSR window, which is a declared stub and answers
//!   zeros — the smoke pins that boundary rather than claiming a timer,
//! * a heartbeat whose `txfull` field is read back from the UART CSR.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// LiteX UART (rxtx @ +0x00, txfull @ +0x04) on this SoC's 0xE0000000 CSR base.
const UART_RXTX: *mut u32 = 0xE000_1800 as *mut u32;
const UART_TXFULL: *const u32 = 0xE000_1804 as *const u32;

/// The timer block is the 8-bit-CSR `LiteX_Timer` variant, declared as a stub
/// in the chip descriptor: reading it documents that it answers zeros.
const TIMER_CSR: *const u32 = 0xE000_2800 as *const u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_TXFULL) != 0 {}
        core::ptr::write_volatile(UART_RXTX, byte as u32);
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

#[entry]
fn main() -> ! {
    puts("MINERVA LITEX SMOKE\n");
    puts("cpu=rv32i uart=litex\n");

    // Add/shift only: the RV32I toolchain cannot emit multiply/divide here.
    let mut acc: u32 = 1;
    for _ in 0..8 {
        acc = acc.wrapping_add(acc);
    }
    puts("acc=");
    put_hex32(acc);
    putc(b'\n');

    puts("timer=");
    put_hex32(unsafe { core::ptr::read_volatile(TIMER_CSR) });
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        puts("hb=");
        put_dec32(heartbeat);
        puts(" txfull=");
        put_hex32(unsafe { core::ptr::read_volatile(UART_TXFULL) });
        putc(b'\n');
        spin(400_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
