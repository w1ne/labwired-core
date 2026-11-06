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

static mut RESULT: u64 = 0;

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
    let mut sequence = 1u64;
    loop {
        sequence = sequence.wrapping_add(1);
        let product = multiply(sequence, sequence.wrapping_add(16));
        unsafe {
            core::ptr::write_volatile(core::ptr::addr_of_mut!(RESULT), product ^ sequence);
        }
    }
}
