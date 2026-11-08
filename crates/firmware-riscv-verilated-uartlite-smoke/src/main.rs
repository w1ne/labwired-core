// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;
use riscv_rt::entry;

const RESULT: *mut u32 = 0x6000_1000 as *mut u32;
const CTRL_READ: *mut u32 = 0x6000_1004 as *mut u32;
const STAT_IDLE: *mut u32 = 0x6000_1008 as *mut u32;
const STAT_DONE: *mut u32 = 0x6000_100C as *mut u32;
const PASS: *mut u32 = 0x6000_1010 as *mut u32;

// Register map of the Verilated RTL behind Renode's CoSimulatedUART
// (renode-verilator-integration samples/uartlite/top.v).
const UART_BASE: usize = 0x7000_0000;
const UART_RX: *const u32 = UART_BASE as *const u32;
const UART_TX: *mut u32 = (UART_BASE + 0x4) as *mut u32;
const UART_CTRL: *mut u32 = (UART_BASE + 0x8) as *mut u32;
const UART_STAT: *const u32 = (UART_BASE + 0xC) as *const u32;

const STAT_RX_VALID: u32 = 1 << 0;
const STAT_TX_EMPTY: u32 = 1 << 2;
const STAT_TX_FULL: u32 = 1 << 3;
const CTRL_RST_TX: u32 = 1 << 0;
const CTRL_RST_RX: u32 = 1 << 1;

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

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UART_STAT) & STAT_TX_FULL != 0 {}
        write_volatile(UART_TX, u32::from(byte));
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(RESULT, multiply(17, 19));
        // Sentinel, so a run that never reaches the CTRL read cannot pass the
        // "CTRL reads 0" check on zeroed RAM.
        write_volatile(CTRL_READ, 0xDEAD_BEEF);

        // Reset both FIFOs. CTRL is write-only in the RTL (reads 0).
        write_volatile(UART_CTRL, CTRL_RST_TX | CTRL_RST_RX);
        let ctrl = read_volatile(UART_CTRL as *const u32);
        write_volatile(CTRL_READ, ctrl);

        let idle = read_volatile(UART_STAT);
        write_volatile(STAT_IDLE, idle);

        for &b in b"UARTLite cosim ok\r\n" {
            putc(b);
        }
        while read_volatile(UART_STAT) & STAT_TX_EMPTY == 0 {}
        let done = read_volatile(UART_STAT);
        write_volatile(STAT_DONE, done);

        // No RX byte is queued, so RX valid must be clear and RX reads 0.
        let rx = read_volatile(UART_RX);
        if ctrl == 0 && idle == STAT_TX_EMPTY && done & STAT_RX_VALID == 0 && rx == 0 {
            write_volatile(PASS, 0x0A11_7E01);
        }
    }
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
