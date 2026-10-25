// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Digilent Arty A7-35T (LiteX VexRiscv SoC) smoke firmware.
//!
//! Boots from the SoC's integrated SRAM (fast boot: the ELF is loaded
//! directly, the LiteX BIOS and the SPI-flash load are skipped) and prints
//! over the LiteX UART at 0x60001800:
//!
//! * a banner and the modelled-surface summary,
//! * the four user LEDs as a walking pattern, each step READ BACK from the
//!   Control-and-Status `Leds` register (so the assertion sees the CSR, not
//!   what the firmware intended to write),
//! * the idle switch/button levels and the button pending/enable registers,
//! * a one-shot run of the modelled LiteX timer (`timer=running` → `timer=zero`),
//! * a heartbeat whose `txfull` field is read back from the UART CSR.
//!
//! Built for `riscv32im-unknown-none-elf` (VexRiscv `rv32im_zicsr_zifencei`).

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// LiteX UART (rxtx @ +0x00, txfull @ +0x04).
const UART_RXTX: *mut u32 = 0x6000_1800 as *mut u32;
const UART_TXFULL: *const u32 = 0x6000_1804 as *const u32;

/// LiteX timer (as on the SoC: load/reload/en/update_value/value + events).
const TIMER_LOAD: *mut u32 = 0x6000_2800 as *mut u32;
const TIMER_RELOAD: *mut u32 = 0x6000_2804 as *mut u32;
const TIMER_EN: *mut u32 = 0x6000_2808 as *mut u32;
const TIMER_EV_STATUS: *const u32 = 0x6000_2814 as *const u32;
const TIMER_EV_PENDING: *mut u32 = 0x6000_2818 as *mut u32;

/// Control-and-status block: Leds @ +0x00 (R/W), Switches @ +0x04,
/// ButtonsStatus @ +0x08, ButtonsPending @ +0x0C (W1C), ButtonsEnabled @ +0x10.
const CAS_LEDS: *mut u32 = 0x6000_6800 as *mut u32;
const CAS_SWITCHES: *const u32 = 0x6000_6804 as *const u32;
const CAS_BUTTONS: *const u32 = 0x6000_6808 as *const u32;
const CAS_PENDING: *mut u32 = 0x6000_680C as *mut u32;
const CAS_ENABLED: *mut u32 = 0x6000_6810 as *mut u32;

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

fn put_hex32(value: u32) {
    for shift in (0..8).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

fn put_dec32(value: u32) {
    let mut digits = [0u8; 10];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
    }
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Drive one LED pattern and print the register readback.
fn leds(pattern: u32) {
    unsafe {
        core::ptr::write_volatile(CAS_LEDS, pattern);
        puts("leds=");
        put_hex32(core::ptr::read_volatile(CAS_LEDS));
        putc(b'\n');
    }
}

/// Run the LiteX timer as a one-shot and wait for its zero event.
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
    puts("ARTY LITEX SMOKE\n");
    puts("cpu=rv32im cas=litex\n");

    // Walking the four user LEDs, each step read back from the CSR.
    for bit in 0..4 {
        leds(1 << bit);
    }
    leds(0xF);
    leds(0);

    // Idle switches/buttons and the button interrupt registers.
    unsafe {
        puts("sw=");
        put_hex32(core::ptr::read_volatile(CAS_SWITCHES));
        puts(" btn=");
        put_hex32(core::ptr::read_volatile(CAS_BUTTONS));
        putc(b'\n');
        // Clear any stale presses, then enable all four button interrupts and
        // read both registers back.
        core::ptr::write_volatile(CAS_PENDING, 0xF);
        core::ptr::write_volatile(CAS_ENABLED, 0xF);
        puts("pend=");
        put_hex32(core::ptr::read_volatile(CAS_PENDING));
        puts(" en=");
        put_hex32(core::ptr::read_volatile(CAS_ENABLED));
        putc(b'\n');
    }

    timer_one_shot(100_000);

    let mut heartbeat: u32 = 0;
    loop {
        puts("hb=");
        put_dec32(heartbeat);
        puts(" txfull=");
        put_hex32(unsafe { core::ptr::read_volatile(UART_TXFULL) });
        putc(b'\n');
        spin(400_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
