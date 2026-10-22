// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! LiteX PicoRV32 SoC smoke firmware.
//!
//! Boots from the SoC's integrated SRAM (fast boot: the ELF is loaded
//! directly, the LiteX BIOS and the SPI-flash load are skipped) and prints
//! over the LiteX UART at 0x60001800:
//!
//! * a banner and the modelled-surface summary,
//! * a multiply result — the PicoRV32 variant in the descriptor is RV32IMC,
//!   so the firmware is built for `riscv32imc-unknown-none-elf` and the
//!   toolchain emits the M-extension instruction,
//! * a readback of the timer CSR window, which is a declared stub (this
//!   platform uses the 8-bit-CSR `LiteX_Timer` variant) and answers zeros,
//! * a heartbeat whose `txfull` field is read back from the UART CSR.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// LiteX UART (rxtx @ +0x00, txfull @ +0x04) on the 0x60000000 CSR map.
const UART_RXTX: *mut u32 = 0x6000_1800 as *mut u32;
const UART_TXFULL: *const u32 = 0x6000_1804 as *const u32;

/// The 8-bit-CSR LiteX timer variant, declared a stub in the chip descriptor.
const TIMER_CSR: *const u32 = 0xE000_2000 as *const u32;

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
    puts("PICORV32 LITEX SMOKE\n");
    puts("cpu=rv32imc uart=litex\n");

    // The M-extension is part of this core: a real multiply, not a shift loop.
    let product: u32 = 7u32.wrapping_mul(6);
    puts("mul=");
    put_hex32(product);
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
