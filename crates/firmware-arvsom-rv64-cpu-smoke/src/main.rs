// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RV64IMAC smoke for the imported ARV-SOM profile. Renode models the board as
//! the StarFive JH7100 CPU platform plus DDR, so besides the DDR result word it
//! polls the wide-register JH7100 UART0 and prints the product. It makes no
//! Linux, floating-point, supervisor-mode, or interrupt claim.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

const RESULT_ADDRESS: usize = 0x8070_0000;

/// JH7100 UART0 at 0x11870000: THR@+0 and LSR@+0x14, 32-bit wide registers.
const UART0_THR: *mut u32 = 0x1187_0000 as *mut u32;
const UART0_LSR: *const u32 = 0x1187_0014 as *const u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART0_LSR) & (1 << 5) == 0 {}
        core::ptr::write_volatile(UART0_THR, byte as u32);
    }
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

#[inline(never)]
fn multiply(left: u64, right: u64) -> u64 {
    let result: u64;
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
    // This product exercises RV64 multiplication with nonzero high halves.
    // Keep the result in a fixed word inside the narrowed 8 MiB RAM window so
    // the onboarding smoke can observe the executed instruction.
    let product = multiply(0x0000_0001_0000_0001, 0x0000_0002_0000_0003);
    unsafe {
        core::ptr::write_volatile(RESULT_ADDRESS as *mut u32, product as u32);
        core::ptr::write_volatile((RESULT_ADDRESS + 4) as *mut u32, (product >> 32) as u32);
    }
    puts("ARV-SOM JH7100 UART0 ok; mul=");
    for shift in (0..16).rev() {
        let nibble = ((product >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
    putc(b'\n');
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
