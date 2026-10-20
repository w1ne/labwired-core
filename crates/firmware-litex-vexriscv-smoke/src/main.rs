// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! LiteX VexRiscv SoC smoke firmware.
//!
//! Boots from the SoC's integrated SRAM (fast boot: the ELF is loaded
//! directly, the LiteX BIOS and the SPI-flash load are skipped) and prints
//! over the LiteX UART at 0x60001800:
//!
//! * a banner and the modelled-surface summary,
//! * a readback of an *unmodelled* CSR window (the SPI-flash controller) —
//!   a declared stub, so it answers zeros; the smoke pins that behaviour so
//!   the declared-vs-modelled boundary stays visible on the wire,
//! * a one-shot run of the modelled LiteX timer: `timer=running` while the
//!   countdown is live, `timer=zero` when the zero event latches,
//! * a heartbeat whose `txfull` field is READ BACK from the UART CSR after
//!   the TX-ready poll, so the assertion sees the register the firmware
//!   actually observed.
//!
//! Built for `riscv32im-unknown-none-elf`: the VexRiscv variant in the
//! descriptor is `rv32im_zicsr_zifencei` (no compressed, no atomics).

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

/// LiteX UART CSR window (rxtx @ +0x00, txfull @ +0x04; see the chip
/// descriptor for the register-layout source).
const UART_RXTX: *mut u32 = 0x6000_1800 as *mut u32;
const UART_TXFULL: *const u32 = 0x6000_1804 as *const u32;

/// LiteX timer CSR window (`litex/soc/cores/timer.py`): load @ +0x00,
/// reload @ +0x04, en @ +0x08, update_value @ +0x0C, value @ +0x10,
/// ev_status @ +0x14, ev_pending @ +0x18, ev_enable @ +0x1C.
const TIMER_LOAD: *mut u32 = 0x6000_2800 as *mut u32;
const TIMER_RELOAD: *mut u32 = 0x6000_2804 as *mut u32;
const TIMER_EN: *mut u32 = 0x6000_2808 as *mut u32;
const TIMER_EV_STATUS: *const u32 = 0x6000_2814 as *const u32;
const TIMER_EV_PENDING: *mut u32 = 0x6000_2818 as *mut u32;

/// An unmodelled LiteX CSR window (the SPI-flash controller at 0x60005000).
/// Reading it documents the declared-stub behaviour: an unmodelled window
/// answers zeros, and the smoke says so on the wire.
const SPI_CSR: *const u32 = 0x6000_5000 as *const u32;

/// TX one byte through the LiteX UART: poll `txfull` until clear (idle 0 =
/// ready), then write `rxtx`.
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

/// Fixed-width lowercase hex, MSB first.
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

/// A busy-wait the optimizer cannot delete: each iteration is an opaque
/// `nop`, and the function is never inlined.
#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Run the LiteX timer as a one-shot and wait for its zero event. The event
/// only latches if the countdown actually ran, so `timer=zero` is evidence
/// for the model rather than decoration.
fn timer_one_shot(cycles: u32) {
    unsafe {
        // Clear any stale event, then stage and arm a one-shot.
        core::ptr::write_volatile(TIMER_EV_PENDING, 1);
        core::ptr::write_volatile(TIMER_LOAD, cycles);
        core::ptr::write_volatile(TIMER_RELOAD, 0);
        core::ptr::write_volatile(TIMER_EN, 1);

        // ev_status is the event level (`counter == 0`): clear means the
        // countdown is live.
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
    puts("LITEX VEXRISCV SMOKE\n");
    puts("cpu=rv32im uart=litex\n");

    // The SPI controller is declared, not modelled: it answers zeros.
    puts("spi=");
    put_hex32(unsafe { core::ptr::read_volatile(SPI_CSR) });
    putc(b'\n');

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
