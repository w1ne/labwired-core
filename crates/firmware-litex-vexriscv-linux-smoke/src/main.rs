// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bare-metal RV32IM UART and timer smoke for imported LiteX VexRiscv profiles.
//! The SD-card platform variant runs directly from DDR and exercises UART only;
//! neither variant boots Linux or exercises SD-card storage.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

const UART_RXTX: *mut u32 = 0xF000_1000 as *mut u32;
const UART_TXFULL: *const u32 = 0xF000_1004 as *const u32;
#[cfg(not(feature = "sdcard-ddr-profile"))]
const TIMER_LOAD: *mut u32 = 0xF000_1800 as *mut u32;
#[cfg(not(feature = "sdcard-ddr-profile"))]
const TIMER_RELOAD: *mut u32 = 0xF000_1804 as *mut u32;
#[cfg(not(feature = "sdcard-ddr-profile"))]
const TIMER_EN: *mut u32 = 0xF000_1808 as *mut u32;
#[cfg(not(feature = "sdcard-ddr-profile"))]
const TIMER_EV_STATUS: *const u32 = 0xF000_1814 as *const u32;
#[cfg(not(feature = "sdcard-ddr-profile"))]
const TIMER_EV_PENDING: *mut u32 = 0xF000_1818 as *mut u32;

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

#[cfg(not(feature = "sdcard-ddr-profile"))]
fn timer_one_shot(cycles: u32) {
    unsafe {
        core::ptr::write_volatile(TIMER_EV_PENDING, 1);
        core::ptr::write_volatile(TIMER_LOAD, cycles);
        core::ptr::write_volatile(TIMER_RELOAD, 0);
        core::ptr::write_volatile(TIMER_EN, 1);

        if core::ptr::read_volatile(TIMER_EV_STATUS) & 1 == 0 {
            puts("timer=running\n");
        } else {
            puts("timer=stuck\n");
        }

        while core::ptr::read_volatile(TIMER_EV_PENDING) & 1 == 0 {}
        puts("timer=zero\n");
    }
}

#[entry]
fn main() -> ! {
    #[cfg(feature = "sdcard-ddr-profile")]
    {
        puts("LITEX VEXRISCV SDCARD PROFILE RV32IMA\n");
        puts("boot=direct-elf memory=ddr uart=litex\n");
    }
    #[cfg(not(feature = "sdcard-ddr-profile"))]
    {
        puts("LITEX VEXRISCV LINUX PROFILE RV32IM\n");
        puts("boot=direct-elf memory=rom+sram\n");
        timer_one_shot(20_000);
    }

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
