// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// R7FA2E1A9 (EK-RA2E1) smoke: Cortex-M23 MULS, SCI9 polled async TX and
// PORT9 output on P915 (FSP ra2e1_ek LED1). Bare-register driver, no HAL.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RESULT: *mut u32 = 0x2000_4000 as *mut u32;
const SCR_READBACK: *mut u32 = 0x2000_4004 as *mut u32;
const PODR_READBACK: *mut u32 = 0x2000_4008 as *mut u32;

// SCI9 @ 0x40070120 (Renode renesas-r7fa2e1a9.repl): SCR @ +0x02,
// TDR @ +0x03, SSR @ +0x04 (TDRE bit 7, TEND bit 2).
const SCI9_SCR: *mut u8 = 0x4007_0122 as *mut u8;
const SCI9_TDR: *mut u8 = 0x4007_0123 as *mut u8;
const SCI9_SSR: *const u8 = 0x4007_0124 as *const u8;
const SCR_TE: u8 = 1 << 5;
const SSR_TDRE: u8 = 1 << 7;
const SSR_TEND: u8 = 1 << 2;

// PORT9 @ 0x40040120: PCNTR1 (PODR[31:16] | PDR[15:0]) @ +0x00,
// PCNTR3 (PORR[31:16] | POSR[15:0]) @ +0x08.
const PORT9_PCNTR1: *mut u32 = 0x4004_0120 as *mut u32;
const PORT9_PCNTR3: *mut u32 = 0x4004_0128 as *mut u32;
const LED1: u32 = 1 << 15; // P915

fn tx(byte: u8) {
    unsafe {
        while read_volatile(SCI9_SSR) & SSR_TDRE == 0 {}
        write_volatile(SCI9_TDR, byte);
    }
}

#[entry]
fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "muls {product}, {right}",
            product = inout(reg) 17u32 => product,
            right = in(reg) 19u32,
            options(nomem, nostack),
        );
        write_volatile(RESULT, product);

        write_volatile(SCI9_SCR, SCR_TE);
        write_volatile(SCR_READBACK, u32::from(read_volatile(SCI9_SCR)));
        for &b in b"RA2E1 SCI9 SMOKE\n" {
            tx(b);
        }
        while read_volatile(SCI9_SSR) & SSR_TEND == 0 {}

        write_volatile(PORT9_PCNTR1, read_volatile(PORT9_PCNTR1) | LED1);
        write_volatile(PORT9_PCNTR3, LED1);
        write_volatile(PODR_READBACK, read_volatile(PORT9_PCNTR1) >> 16);
    }

    let mut on = true;
    loop {
        for _ in 0..2000 {
            unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
        }
        on = !on;
        // POSR sets, PORR clears.
        let v = if on { LED1 } else { LED1 << 16 };
        unsafe { write_volatile(PORT9_PCNTR3, v) };
    }
}
