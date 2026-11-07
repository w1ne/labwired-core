// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Direct-ELF RV32IMAC smoke against the imported LiteX Zephyr/I2S platform.
//! The fixture exercises CPU arithmetic, one RAM word, and LiteX UART only.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::panic::PanicInfo;

const UART_RXTX: *mut u32 = 0x8200_1800 as *mut u32;
const UART_TXFULL: *const u32 = 0x8200_1804 as *const u32;
const ATOMIC_WORD: *mut u32 = 0x4001_0000 as *mut u32;

core::arch::global_asm!(
    ".section .text.entry,\"ax\",@progbits",
    ".globl _start",
    ".type _start,@function",
    "_start:",
    "la sp, __stack_top",
    "call main",
    "1: j 1b",
);

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_TXFULL) != 0 {}
        core::ptr::write_volatile(UART_RXTX, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for &byte in text {
        putc(byte);
    }
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    let product: u32;
    let previous: u32;
    unsafe {
        core::arch::asm!("c.nop");
        core::arch::asm!(
            "mul {product}, {left}, {right}",
            product = lateout(reg) product,
            left = in(reg) 17u32,
            right = in(reg) 19u32,
            options(nomem, nostack),
        );
        core::ptr::write_volatile(ATOMIC_WORD, 19);
        core::arch::asm!(
            "amoadd.w {previous}, {addend}, ({address})",
            previous = lateout(reg) previous,
            addend = in(reg) 304u32,
            address = in(reg) ATOMIC_WORD,
            options(nostack),
        );
    }

    if product == 323 && previous == 19 {
        puts(b"LITEX ZEPHYR RV32IMAC UART mul=323 amo=19\n");
    } else {
        puts(b"LITEX ZEPHYR RV32IMAC arithmetic failed\n");
    }

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
