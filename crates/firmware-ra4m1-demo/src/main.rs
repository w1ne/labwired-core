// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// R7FA4M1AB smoke firmware for Arduino Uno R4 Minima: ensure HOCO running,
// print OK\n via SCI2 TDR, toggle D13 (P111). Bare-register driver — no HAL.
// Same binary targets silicon and the LabWired model.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// R_SYSTEM @ 0x4001E000 — HOCOCR @ 0x36, OSCSF @ 0x3C (R01UH0887 / R7FA4M1AB.h).
const HOCOCR: *mut u8 = 0x4001_E036 as *mut u8;
const OSCSF: *mut u8 = 0x4001_E03C as *mut u8;
const HCSTP: u8 = 1 << 0;
const HOCOSF: u8 = 1 << 0;

// SCI2 @ 0x40070040 — TDR @ +0x03 (classic SCI, async non-FIFO).
const SCI2_TDR: *mut u8 = 0x4007_0043 as *mut u8;

// PORT1 @ 0x40040020 — PCNTR1 (PDR/PODR) @ +0x00, PCNTR3 (POSR/PORR) @ +0x08.
const PORT1_PCNTR1: *mut u32 = 0x4004_0020 as *mut u32;
const PORT1_PCNTR3: *mut u32 = 0x4004_0028 as *mut u32;

const LED: u32 = 1 << 11; // P111 — D13 / LED_BUILTIN

#[entry]
fn main() -> ! {
    unsafe {
        // HOCO is typically already running out of reset (OFS1); clear HCSTP
        // and wait for OSCSF.HOCOSF in case firmware or OFS1 left it stopped.
        write_volatile(HOCOCR, 0);
        while read_volatile(OSCSF) & HOCOSF == 0 {
            if read_volatile(HOCOCR) & HCSTP != 0 {
                write_volatile(HOCOCR, 0);
            }
        }

        // P111 output: set PDR[11] via PCNTR1 low half.
        let pcntr1 = read_volatile(PORT1_PCNTR1);
        write_volatile(PORT1_PCNTR1, pcntr1 | LED);

        for b in b"OK\n" {
            write_volatile(SCI2_TDR, *b);
        }

        // Toggle LED via POSR (set PODR[11]).
        write_volatile(PORT1_PCNTR3, LED);
    }
    loop {}
}
