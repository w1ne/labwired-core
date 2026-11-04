// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// PicoSoC SimpleUART `reg_dat` register from Renode's picosoc.repl.
const UART_DATA: *mut u32 = 0x0200_0004 as *mut u32;

fn putc(byte: u8) {
    unsafe { core::ptr::write_volatile(UART_DATA, u32::from(byte)) };
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

#[entry]
fn main() -> ! {
    puts("PICOSOC RV32IMC SIMPLEUART SMOKE\n");
    puts("uart=0x02000004\n");
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
