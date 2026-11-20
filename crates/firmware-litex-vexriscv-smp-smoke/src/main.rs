// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Single-hart RV32IM and LiteX UART smoke for the imported VexRiscv SMP
//! platform. It does not exercise atomics, SMP, or CLINT interrupts.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

const UART_RXTX: *mut u32 = 0xF000_1800 as *mut u32;
const UART_TXFULL: *const u32 = 0xF000_1804 as *const u32;

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

#[entry]
fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "mul {product}, {left}, {right}",
            product = out(reg) product,
            left = in(reg) 19u32,
            right = in(reg) 11u32,
            options(nomem, nostack, preserves_flags),
        );
    }

    puts("LITEX VEXRISCV SMP SINGLE-HART SMOKE\n");
    puts("cpu=rv32im hart=0 uart=litex\n");
    puts("mul=");
    put_hex32(product);
    putc(b'\n');

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
