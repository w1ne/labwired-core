// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Nano 33 BLE smoke firmware.
//!
//! The board carries an nRF52840 (Cortex-M4F): 1 MiB flash at 0x0, 256 KiB RAM
//! at 0x20000000. `Serial1` is UARTE0 on D0/TX = P1.03 and D1/RX = P1.10; the
//! board's USB CDC (`Serial`) is not modelled. The RGB LED is **active low**
//! (Arduino's own documentation: "the RGB pins are active-low"), wired to
//! P0.24 (red) / P0.16 (green) / P0.06 (blue); the built-in D13 LED is P0.13
//! and active high.
//!
//! Prints:
//! * a banner and the modelled-surface summary,
//! * the RGB sequence with the GPIO0 OUT register read back from the model
//!   (`rgb=red out=00012040` — red's bit cleared, the other channels set),
//! * a heartbeat with the same readback.
//!
//! The onboard LSM9DS1 IMU (TWI0, 0x6B/0x1E) is not modelled; nothing here
//! touches the bus, and VALIDATION.md records that boundary.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

/// UARTE0 — the nRF52840 console is EasyDMA-based (no byte-at-a-time TXD).
const UART0_BASE: u32 = 0x4000_2000;
const UART0_TASKS_STARTTX: *mut u32 = (UART0_BASE + 0x008) as *mut u32;
const UART0_EVENTS_ENDTX: *mut u32 = (UART0_BASE + 0x120) as *mut u32;
const UART0_ENABLE: *mut u32 = (UART0_BASE + 0x500) as *mut u32;
const UART0_PSEL_TXD: *mut u32 = (UART0_BASE + 0x50C) as *mut u32;
const UART0_PSEL_RXD: *mut u32 = (UART0_BASE + 0x514) as *mut u32;
const UART0_BAUDRATE: *mut u32 = (UART0_BASE + 0x524) as *mut u32;
const UART0_TXD_PTR: *mut u32 = (UART0_BASE + 0x544) as *mut u32;
const UART0_TXD_MAXCNT: *mut u32 = (UART0_BASE + 0x548) as *mut u32;
const UARTE_ENABLE: u32 = 8; // 8 = UARTE (EasyDMA); 4 is the legacy UART
const BAUDRATE_115200: u32 = 0x01D7_E000;

/// GPIO0 — OUT @ +0x504, OUTSET @ +0x508, OUTCLR @ +0x50C, DIRSET @ +0x518.
const GPIO0_BASE: u32 = 0x5000_0000;
const GPIO0_OUT: *const u32 = (GPIO0_BASE + 0x504) as *const u32;
const GPIO0_OUTSET: *mut u32 = (GPIO0_BASE + 0x508) as *mut u32;
const GPIO0_OUTCLR: *mut u32 = (GPIO0_BASE + 0x50C) as *mut u32;
const GPIO0_DIRSET: *mut u32 = (GPIO0_BASE + 0x518) as *mut u32;

const LED_RED: u32 = 1 << 24;
const LED_GREEN: u32 = 1 << 16;
const LED_BLUE: u32 = 1 << 6;
const LED_BUILTIN: u32 = 1 << 13; // D13, active high

fn tx(bytes: &[u8]) {
    // EasyDMA fetches the buffer from RAM (not flash), so copy it to the stack.
    let mut buffer = [0u8; 64];
    let len = bytes.len().min(buffer.len());
    buffer[..len].copy_from_slice(&bytes[..len]);
    unsafe {
        write_volatile(UART0_EVENTS_ENDTX, 0);
        write_volatile(UART0_TXD_PTR, buffer.as_ptr() as u32);
        write_volatile(UART0_TXD_MAXCNT, len as u32);
        write_volatile(UART0_TASKS_STARTTX, 1);
        while read_volatile(UART0_EVENTS_ENDTX) == 0 {}
    }
}

fn put_hex32(value: u32) {
    let mut digits = [b'0'; 8];
    for (index, digit) in digits.iter_mut().enumerate() {
        let nibble = ((value >> ((7 - index) * 4)) & 0xF) as u8;
        *digit = match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        };
    }
    tx(&digits);
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
    tx(&digits[start..]);
}

fn out_readback() -> u32 {
    unsafe { read_volatile(GPIO0_OUT) }
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(UART0_ENABLE, UARTE_ENABLE);
        write_volatile(UART0_PSEL_TXD, 32 + 3); // P1.03 = D0/TX
        write_volatile(UART0_PSEL_RXD, 32 + 10); // P1.10 = D1/RX
        write_volatile(UART0_BAUDRATE, BAUDRATE_115200);

        write_volatile(GPIO0_DIRSET, LED_RED | LED_GREEN | LED_BLUE | LED_BUILTIN);
        // RGB off (active low), built-in LED on (active high).
        write_volatile(GPIO0_OUTSET, LED_RED | LED_GREEN | LED_BLUE);
        write_volatile(GPIO0_OUTSET, LED_BUILTIN);
    }

    tx(b"ARDUINO NANO 33 BLE SMOKE\n");
    tx(b"cpu=nrf52840 uart=uarte0\n");

    unsafe {
        write_volatile(GPIO0_OUTCLR, LED_RED);
    }
    tx(b"rgb=red out=");
    put_hex32(out_readback());
    tx(b"\n");

    unsafe {
        write_volatile(GPIO0_OUTSET, LED_RED);
        write_volatile(GPIO0_OUTCLR, LED_GREEN);
    }
    tx(b"rgb=green out=");
    put_hex32(out_readback());
    tx(b"\n");

    unsafe {
        write_volatile(GPIO0_OUTSET, LED_GREEN);
        write_volatile(GPIO0_OUTCLR, LED_BLUE);
    }
    tx(b"rgb=blue out=");
    put_hex32(out_readback());
    tx(b"\n");

    let mut heartbeat: u32 = 0;
    loop {
        tx(b"hb=");
        put_dec32(heartbeat);
        tx(b" out=");
        put_hex32(out_readback());
        tx(b"\n");
        for _ in 0..200_000u32 {
            unsafe {
                let _ = read_volatile(GPIO0_OUT);
            }
        }
        heartbeat = heartbeat.wrapping_add(1);
    }
}
