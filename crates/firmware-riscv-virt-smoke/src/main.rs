// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const UART_THR: *mut u8 = 0x1000_0000 as *mut u8;
const UART_DLM: *mut u8 = 0x1000_0001 as *mut u8;
const UART_LCR: *mut u8 = 0x1000_0003 as *mut u8;
const UART_LSR: *const u8 = 0x1000_0005 as *const u8;
const RESULT: *mut u32 = 0x8000_1000 as *mut u32;

fn uart_init() {
    unsafe {
        // Program a nonzero divisor and 8-N-1 before polling THRE and sending.
        core::ptr::write_volatile(UART_LCR, 0x80);
        core::ptr::write_volatile(UART_THR, 1);
        core::ptr::write_volatile(UART_DLM, 0);
        core::ptr::write_volatile(UART_LCR, 0x03);
    }
}

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART_LSR) & 0x20 == 0 {}
        core::ptr::write_volatile(UART_THR, byte);
    }
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
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
    uart_init();
    unsafe { core::arch::asm!("c.nop", options(nomem, nostack, preserves_flags)) };
    unsafe { core::ptr::write_volatile(RESULT, multiply(17, 19)) };
    puts("RV32 VIRT\n");
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
