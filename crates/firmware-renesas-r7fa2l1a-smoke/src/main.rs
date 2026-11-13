// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// R7FA2L1A (EK-RA2L1) smoke: Cortex-M23 MULS, SCI3 polled async TX and
// PORT5 output on P503 (FSP ra2l1_ek LED1). Bare-register driver, no HAL.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RESULT: *mut u32 = 0x2000_0000 as *mut u32;
const SCR_READBACK: *mut u32 = 0x2000_0004 as *mut u32;
const PODR_READBACK: *mut u32 = 0x2000_0008 as *mut u32;

// SCI3 @ 0x40070060 (Renode renesas-r7fa2l1a.repl): SCR @ +0x02,
// TDR @ +0x03, SSR @ +0x04 (TDRE bit 7, TEND bit 2).
const SCI3_SCR: *mut u8 = 0x4007_0062 as *mut u8;
const SCI3_TDR: *mut u8 = 0x4007_0063 as *mut u8;
const SCI3_SSR: *const u8 = 0x4007_0064 as *const u8;
const SCR_TE: u8 = 1 << 5;
const SSR_TDRE: u8 = 1 << 7;
const SSR_TEND: u8 = 1 << 2;

// PORT5 @ 0x400400A0: PCNTR1 (PODR[31:16] | PDR[15:0]) @ +0x00,
// PCNTR3 (PORR[31:16] | POSR[15:0]) @ +0x08.
const PORT5_PCNTR1: *mut u32 = 0x4004_00A0 as *mut u32;
const PORT5_PCNTR3: *mut u32 = 0x4004_00A8 as *mut u32;
const LED1: u32 = 1 << 3; // P503

fn tx(byte: u8) {
    unsafe {
        while read_volatile(SCI3_SSR) & SSR_TDRE == 0 {}
        write_volatile(SCI3_TDR, byte);
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

        write_volatile(SCI3_SCR, SCR_TE);
        write_volatile(SCR_READBACK, u32::from(read_volatile(SCI3_SCR)));
        for &b in b"RA2L1 SCI3 SMOKE\n" {
            tx(b);
        }
        while read_volatile(SCI3_SSR) & SSR_TEND == 0 {}

        write_volatile(PORT5_PCNTR1, read_volatile(PORT5_PCNTR1) | LED1);
        write_volatile(PORT5_PCNTR3, LED1);
        write_volatile(PODR_READBACK, read_volatile(PORT5_PCNTR1) >> 16);
    }

    let mut on = true;
    loop {
        for _ in 0..2000 {
            unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
        }
        on = !on;
        // POSR sets, PORR clears.
        let v = if on { LED1 } else { LED1 << 16 };
        unsafe { write_volatile(PORT5_PCNTR3, v) };
    }
}
