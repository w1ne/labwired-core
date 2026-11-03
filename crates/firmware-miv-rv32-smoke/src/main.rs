// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Mi-V RV32 smoke: CPU execution, CoreUART TX-ready polling, and UART output.
//!
//! The ELF is loaded directly in the Renode Mi-V flash window. The smoke does
//! not claim CLINT, PLIC, GPIO, timer, DDR initialization, or a boot-ROM path.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// Microchip CoreUARTapb at 0x70001000: TxData@+0x00, Status@+0x10;
/// TXRDY is bit 0 and is set while the transmitter can accept a byte.
const UART_TXDATA: *mut u32 = 0x7000_1000 as *mut u32;
const UART_STATUS: *const u32 = 0x7000_1010 as *const u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_STATUS) & 1 == 0 {}
        core::ptr::write_volatile(UART_TXDATA, byte as u32);
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

#[inline(never)]
fn multiply(left: u32, right: u32) -> u32 {
    let result: u32;
    unsafe {
        core::arch::asm!(
            "mul {result}, {left}, {right}",
            result = lateout(reg) result,
            left = in(reg) left,
            right = in(reg) right,
            options(nomem, nostack, preserves_flags),
        );
    }
    result
}

#[entry]
fn main() -> ! {
    puts("MIV RV32 SMOKE\n");
    puts("cpu=rv32im uart=coreuart\n");
    puts("mul=");
    put_hex32(multiply(17, 19));
    putc(b'\n');

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
