// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// KW41Z smoke firmware: bring up LPUART0 and print a known banner. Boots on a
// real MKW41Z512 (Cortex-M0+) and on the LabWired KW41Z model unchanged — the
// behavioural LPUART (Kinetis layout) routes DATA writes to the TX sink and
// reports TDRE in STAT, so the byte stream below is what both produce.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// LPUART0 register block (MKW41Z4 memory map).
const LPUART0_BASE: u32 = 0x4005_4000;
const LPUART0_STAT: *mut u32 = (LPUART0_BASE + 0x04) as *mut u32; // status
const LPUART0_CTRL: *mut u32 = (LPUART0_BASE + 0x08) as *mut u32; // control
const LPUART0_DATA: *mut u32 = (LPUART0_BASE + 0x0C) as *mut u32; // data

const STAT_TDRE: u32 = 1 << 23; // Transmit Data Register Empty
const CTRL_TE: u32 = 1 << 19; // Transmitter Enable
const CTRL_RE: u32 = 1 << 18; // Receiver Enable

#[entry]
fn main() -> ! {
    unsafe {
        // Enable the transmitter (and receiver, as a typical console would).
        write_volatile(LPUART0_CTRL, CTRL_TE | CTRL_RE);
    }

    print_uart("KW41Z_SMOKE_OK\n");

    loop {
        // Idle. A volatile read keeps the loop from being optimised away
        // without pulling in a cortex-m intrinsic.
        unsafe {
            let _ = read_volatile(LPUART0_STAT);
        }
    }
}

fn print_uart(s: &str) {
    for b in s.bytes() {
        unsafe {
            // Wait for the transmit data register to drain (TDRE), exactly as
            // the NXP HAL does, then push the byte.
            while read_volatile(LPUART0_STAT) & STAT_TDRE == 0 {}
            write_volatile(LPUART0_DATA, b as u32);
        }
    }
}
