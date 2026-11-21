// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Direct firmware smoke for the bounded OpenTitan Earl Grey profile.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// OpenTitan UART STATUS@+0x14 and WDATA@+0x1C.
const UART0_STATUS: *const u32 = 0x4000_0014 as *const u32;
const UART0_WDATA: *mut u32 = 0x4000_001C as *mut u32;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART0_STATUS) & 1 != 0 {}
        core::ptr::write_volatile(UART0_WDATA, byte as u32);
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
    puts("OPENTITAN EARLGREY SMOKE\n");
    puts("core=rv32imc uart0=opentitan\n");
    puts("mul=");
    put_hex32(multiply(17, 19));
    putc(b'\n');

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
