// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Vega board (RI5CY) smoke firmware.
//!
//! The board's SoC maps 1 MiB of flash at 0x0, 192 KiB of SRAM at 0x20000000
//! and an NXP LPUART0 at 0x40042000. The ELF is loaded directly (fast boot).
//!
//! Prints:
//! * a banner and the modelled-surface summary,
//! * a real multiply (`mul=0000002a`) — the RI5CY core is RV32IMC, so the
//!   toolchain emits the M extension,
//! * readbacks of the SCG and PCC windows, which are declared stubs and
//!   answer zeros — the smoke pins that boundary,
//! * a heartbeat whose `stat=` field is read back from the LPUART status
//!   register (0x00c00000 = TDRE|TC idle).

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// NXP LPUART0 on the RI5CY SoC: DATA @ +0x0C (write transmits), STAT @ +0x04
/// (TDRE bit 23, TC bit 22).
const LPUART_DATA: *mut u32 = 0x4004_200C as *mut u32;
const LPUART_STAT: *const u32 = 0x4004_2004 as *const u32;
const LPUART_TDRE: u32 = 1 << 23;

/// Declared stubs: the System Clock Generator and the two PCC clock banks.
const SCG_CSR: *const u32 = 0x4002_C010 as *const u32;
const PCC0: *const u32 = 0x4002_B000 as *const u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(LPUART_STAT) & LPUART_TDRE == 0 {}
        core::ptr::write_volatile(LPUART_DATA, byte as u32);
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
    puts("VEGABOARD RI5CY SMOKE\n");
    puts("cpu=rv32imc uart=lpuart\n");

    let product: u32 = 7u32.wrapping_mul(6);
    puts("mul=");
    put_hex32(product);
    putc(b'\n');

    puts("scg=");
    put_hex32(unsafe { core::ptr::read_volatile(SCG_CSR) });
    puts(" pcc0=");
    put_hex32(unsafe { core::ptr::read_volatile(PCC0) });
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        puts("hb=");
        put_dec32(heartbeat);
        puts(" stat=");
        put_hex32(unsafe { core::ptr::read_volatile(LPUART_STAT) });
        putc(b'\n');
        spin(400_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
