// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CPU-only RV64IMAC smoke for the imported ARV-SOM profile. It performs no
//! peripheral accesses and makes no Linux, floating-point, or supervisor-mode
//! claim.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

const RESULT_ADDRESS: usize = 0x8070_0000;

#[inline(never)]
fn multiply(left: u64, right: u64) -> u64 {
    let result: u64;
    unsafe {
        core::arch::asm!(
            "mul {result}, {left}, {right}",
            result = lateout(reg) result,
            left = in(reg) left,
            right = in(reg) right,
            options(nomem, nostack, preserves_flags),
        );
    }
    result
}

#[entry]
fn main() -> ! {
    // This product exercises RV64 multiplication with nonzero high halves.
    // Keep the result in a fixed word inside the narrowed 8 MiB RAM window so
    // the onboarding smoke can observe the executed instruction.
    let product = multiply(0x0000_0001_0000_0001, 0x0000_0002_0000_0003);
    unsafe {
        core::ptr::write_volatile(RESULT_ADDRESS as *mut u32, product as u32);
        core::ptr::write_volatile((RESULT_ADDRESS + 4) as *mut u32, (product >> 32) as u32);
    }
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
