// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Arduino UNO R4 WiFi (R7FA4M1AB3CFM#AA0) focused register-level smoke.
// Exercises SCI2 on D0/D1 and the D13/LED_BUILTIN GPIO at P102.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const HOCOCR: *mut u8 = 0x4001_E036 as *mut u8;
const OSCSF: *const u8 = 0x4001_E03C as *const u8;
const HCSTP: u8 = 1 << 0;
const HOCOSF: u8 = 1 << 0;

const PFS_P102: *mut u8 = 0x4004_0848 as *mut u8;
const PFS_PWPR: *mut u8 = 0x4004_0D03 as *mut u8;
const PORT1_PCNTR1: *mut u32 = 0x4004_0020 as *mut u32;
const PORT1_PCNTR3: *mut u32 = 0x4004_0028 as *mut u32;
const SCI2_TDR: *mut u8 = 0x4007_0043 as *mut u8;

const LED_D13: u32 = 1 << 2; // P102 / Arduino D13 / LED_BUILTIN
const PWPR_B0WI: u8 = 1 << 7;
const PWPR_PFSWE: u8 = 1 << 6;

fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

fn puts(text: &[u8]) {
    for &byte in text {
        unsafe { write_volatile(SCI2_TDR, byte) };
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Ensure the high-speed on-chip oscillator is running after reset.
        write_volatile(HOCOCR, 0);
        while read_volatile(OSCSF) & HOCOSF == 0 {
            if read_volatile(HOCOCR) & HCSTP != 0 {
                write_volatile(HOCOCR, 0);
            }
        }

        // R_BSP_PinAccessEnable, then configure P102 as a GPIO output.
        write_volatile(PFS_PWPR, 0);
        write_volatile(PFS_PWPR, PWPR_PFSWE);
        write_volatile(PFS_P102, 1 << 2); // PDR=1, GPIO mux (PMR=0, PSEL=0)
        write_volatile(PFS_PWPR, 0);
        write_volatile(PFS_PWPR, PWPR_B0WI);

        // P102 is port 1 bit 2. PCNTR1 low half is PDR; bit 2 selects output.
        let pcntr1 = read_volatile(PORT1_PCNTR1);
        write_volatile(PORT1_PCNTR1, pcntr1 | LED_D13);
    }

    puts(b"UNO_R4_WIFI_OK\n");

    // Repeatedly drive D13 high then low so the smoke observes real GPIO edges.
    for _ in 0..4 {
        unsafe { write_volatile(PORT1_PCNTR3, LED_D13) };
        spin(128);
        unsafe { write_volatile(PORT1_PCNTR3, LED_D13 << 16) };
        spin(128);
    }

    loop {
        core::hint::spin_loop();
    }
}
