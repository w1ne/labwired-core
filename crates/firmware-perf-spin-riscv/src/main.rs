// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RISC-V throughput fixture for `scripts/perf/board_perf.py`.
//!
//! The RISC-V twin of `crates/firmware-perf-spin`: the same bare ALU spin loop,
//! no peripheral touched, so every host instruction the simulator spends on a
//! step is its own overhead. It exists because `cortex-m-rt` cannot link for
//! the ESP32-C3, which left the only RISC-V part in the matrix unmeasured.
//!
//! A number from this fixture is comparable to *this board's own history*,
//! which is what the gate exists to hold. It is not comparable to a Cortex-M
//! board's number — a different ISA means a different instruction mix per
//! simulated step, and no re-linking can make those the same.
//!
//! Built for `riscv32imc-unknown-none-elf` to match the C3's actual ISA
//! (RV32IMC), so the decode path the gate measures is the one real C3 firmware
//! exercises rather than a compressed-instruction-free subset no user ships.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

#[entry]
fn main() -> ! {
    let mut acc: u32 = 0;
    loop {
        // `black_box` keeps LLVM from folding the loop away; the body stays a
        // fixed, branch-predictable add/compare/branch triple, same as the
        // Cortex-M fixture's.
        acc = acc.wrapping_add(1);
        core::hint::black_box(acc);
    }
}
