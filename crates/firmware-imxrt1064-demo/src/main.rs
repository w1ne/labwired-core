// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// i.MX RT106x / Teensy 4.1 smoke: ungating CCM clocks, print OK\n via LPUART6
// DATA, toggle GPIO2 pin 3 (LED) via DR_TOGGLE. Bare-register — no HAL.
// Soft-float thumbv7em-none-eabi. Same binary targets the LabWired model.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// CCM @ 0x400FC000 — CCGR0@0x68 (GPIO2 CG15), CCGR3@0x74 (LPUART6 CG3).
const CCM_CCGR0: *mut u32 = 0x400F_C068 as *mut u32;
const CCM_CCGR3: *mut u32 = 0x400F_C074 as *mut u32;

// LPUART6 @ 0x40198000 — STAT@+0x04, CTRL@+0x08, DATA@+0x0C (MIMXRT1062.h).
const LPUART6_BASE: u32 = 0x4019_8000;
const LPUART6_STAT: *mut u32 = (LPUART6_BASE + 0x04) as *mut u32;
const LPUART6_CTRL: *mut u32 = (LPUART6_BASE + 0x08) as *mut u32;
const LPUART6_DATA: *mut u32 = (LPUART6_BASE + 0x0C) as *mut u32;

const STAT_TDRE: u32 = 1 << 23;
const CTRL_TE: u32 = 1 << 19;
const CTRL_RE: u32 = 1 << 18;

// GPIO2 @ 0x401BC000 — GDIR@+0x04, DR_TOGGLE@+0x8C (IMXRT1060RM §12).
const GPIO2_GDIR: *mut u32 = 0x401B_C004 as *mut u32;
const GPIO2_DR_TOGGLE: *mut u32 = 0x401B_C08C as *mut u32;

const LED: u32 = 1 << 3; // GPIO2_IO03 — Teensy pin 13 / LED_BUILTIN

#[entry]
fn main() -> ! {
    unsafe {
        // Ungate GPIO2 (CCGR0 CG15 bits 31:30) and LPUART6 (CCGR3 CG3 bits 7:6).
        write_volatile(CCM_CCGR0, 0b11 << 30);
        write_volatile(CCM_CCGR3, 0b11 << 6);

        write_volatile(LPUART6_CTRL, CTRL_TE | CTRL_RE);

        // Pin 13 output.
        let gdir = read_volatile(GPIO2_GDIR);
        write_volatile(GPIO2_GDIR, gdir | LED);

        for b in b"OK\n" {
            while read_volatile(LPUART6_STAT) & STAT_TDRE == 0 {}
            write_volatile(LPUART6_DATA, *b as u32);
        }

        write_volatile(GPIO2_DR_TOGGLE, LED);
    }
    loop {}
}
