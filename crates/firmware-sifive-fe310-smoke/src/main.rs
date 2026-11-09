// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SiFive FE310 smoke: RV32IM execution, UART0 TX, and HiFive1 Rev B GPIO.
//!
//! The ELF is loaded into the FE310's QSPI XIP window and entered directly.
//! It checks the imported system's RISC-V core, uart0, and GPIO software path
//! used by the board's active-low green LED. It does not exercise the mask-ROM
//! boot flow, GPIO interrupts/IO functions, PLIC, CLINT or uart1.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// FE310 uart0 at 0x10013000: txdata@+0x00, with bit 31 indicating full.
const UART0_TXDATA: *mut u32 = 0x1001_3000 as *mut u32;
const GPIO_INPUT_VAL: *mut u32 = 0x1001_2000 as *mut u32;
const GPIO_INPUT_EN: *mut u32 = 0x1001_2004 as *mut u32;
const GPIO_OUTPUT_EN: *mut u32 = 0x1001_2008 as *mut u32;
const GPIO_OUTPUT_VAL: *mut u32 = 0x1001_200C as *mut u32;
const GREEN_LED: u32 = 1 << 19;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(UART0_TXDATA) & (1 << 31) != 0 {}
        core::ptr::write_volatile(UART0_TXDATA, byte as u32);
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
    puts("SIFIVE FE310 SMOKE\n");
    puts("cpu=rv32im uart=sifive\n");
    puts("mul=");
    put_hex32(multiply(17, 19));
    putc(b'\n');

    let gpio_pass = unsafe {
        core::ptr::write_volatile(GPIO_INPUT_EN, GREEN_LED);
        core::ptr::write_volatile(GPIO_OUTPUT_VAL, 0);
        core::ptr::write_volatile(GPIO_OUTPUT_EN, GREEN_LED);

        let mut pass = core::ptr::read_volatile(GPIO_INPUT_EN) == GREEN_LED
            && core::ptr::read_volatile(GPIO_OUTPUT_EN) == GREEN_LED;
        for value in [GREEN_LED, 0, GREEN_LED] {
            core::ptr::write_volatile(GPIO_OUTPUT_VAL, value);
            pass &= core::ptr::read_volatile(GPIO_OUTPUT_VAL) == value;
            pass &= core::ptr::read_volatile(GPIO_INPUT_VAL) & GREEN_LED == value;
        }
        pass
    };
    let (input_en, output_en, output_val, input_val) = unsafe {
        (
            core::ptr::read_volatile(GPIO_INPUT_EN),
            core::ptr::read_volatile(GPIO_OUTPUT_EN),
            core::ptr::read_volatile(GPIO_OUTPUT_VAL),
            core::ptr::read_volatile(GPIO_INPUT_VAL),
        )
    };
    puts("gpio=");
    puts(if gpio_pass { "PASS" } else { "FAIL" });
    puts(" input_en=");
    put_hex32(input_en);
    puts(" output_en=");
    put_hex32(output_en);
    puts(" output_val=");
    put_hex32(output_val);
    puts(" input_val=");
    put_hex32(input_val);
    putc(b'\n');

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
