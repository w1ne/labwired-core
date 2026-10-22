// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RISC-V `virt` machine smoke firmware.
//!
//! The virt machine has one flat RAM at 0x80000000 and an NS16550 UART at
//! 0x10000000. The descriptor splits RAM into a 1 MiB code window (flash) and
//! a 7 MiB data/stack window; the ELF is loaded directly (fast boot).
//!
//! Prints:
//! * a banner and the modelled-surface summary,
//! * a real multiply (`mul=0000002a`), so a decoder without the M extension
//!   fails the smoke,
//! * a readback of the PLIC and CLINT windows, which are declared stubs and
//!   answer zeros — the smoke pins that boundary,
//! * a heartbeat whose `lsr=` field is read back from the NS16550 line-status
//!   register (0x60 = THRE|TEMT idle).

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// NS16550 on the virt machine: THR/RBR @ +0x00, LSR @ +0x05 (THRE bit 5).
const UART_THR: *mut u8 = 0x1000_0000 as *mut u8;
const UART_LSR: *const u8 = 0x1000_0005 as *const u8;
const UART_LSR_THRE: u8 = 1 << 5;

/// Declared stubs on the virt machine (no PLIC/CLINT model in the engine yet).
const PLIC_BASE: *const u32 = 0x0C00_0000 as *const u32;
const CLINT_BASE: *const u32 = 0x0200_0000 as *const u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_LSR) & UART_LSR_THRE == 0 {}
        core::ptr::write_volatile(UART_THR, byte);
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

fn put_hex8(value: u8) {
    for shift in (0..2).rev() {
        let nibble = (value >> (shift * 4)) & 0xF;
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
    puts("RISCV VIRT SMOKE\n");
    puts("cpu=rv32imac uart=ns16550\n");

    // The A extension is part of the core; this firmware only needs M.
    let product: u32 = 7u32.wrapping_mul(6);
    puts("mul=");
    put_hex32(product);
    putc(b'\n');

    puts("plic=");
    put_hex32(unsafe { core::ptr::read_volatile(PLIC_BASE) });
    puts(" clint=");
    put_hex32(unsafe { core::ptr::read_volatile(CLINT_BASE) });
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        puts("hb=");
        put_dec32(heartbeat);
        puts(" lsr=");
        put_hex8(unsafe { core::ptr::read_volatile(UART_LSR) });
        putc(b'\n');
        spin(400_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
