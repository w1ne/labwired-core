// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! LiteX VexRiscv GPIOIn probe.
//!
//! Reads the LiteX GPIOIn `in` CSR (Zephyr enjoydigital litex_vexriscv.dts
//! `gpio_in: gpio@e0006000`, ngpios 4) and prints its four pin levels on the
//! LiteX UART as `IN=<p3><p2><p1><p0>`, one line per sample. The line repeats,
//! so a host that changes a pad level sees the next sample follow it.
//!
//! The committed fixture is examples/litex-vexriscv/firmware/
//! litex-vexriscv-gpio-in.elf (rebuild command in examples/litex-vexriscv/
//! VALIDATION.md).

#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const UART_RXTX: *mut u32 = 0x6000_1800 as *mut u32;
const UART_TXFULL: *const u32 = 0x6000_1804 as *const u32;
const GPIO_IN: *const u32 = 0xE000_6000 as *const u32;
const NGPIOS: u32 = 4;

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

#[entry]
fn main() -> ! {
    puts("LITEX GPIO IN\n");
    loop {
        let pins = unsafe { core::ptr::read_volatile(GPIO_IN) };
        puts("IN=");
        for pin in (0..NGPIOS).rev() {
            putc(if pins & (1 << pin) != 0 { b'1' } else { b'0' });
        }
        putc(b'\n');
    }
}
