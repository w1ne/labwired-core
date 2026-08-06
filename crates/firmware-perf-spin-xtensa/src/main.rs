// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Xtensa throughput fixture for `scripts/perf/board_perf.py`.
//!
//! The Xtensa twin of `crates/firmware-perf-spin`: the same bare ALU spin loop,
//! no peripheral touched, so every host instruction the simulator spends on a
//! step is its own overhead.
//!
//! Placement comes from `esp-hal`'s `linkall.x` for the selected chip feature,
//! not from a generated `memory.x` — the ESP parts boot through a layout the
//! HAL already describes correctly, and hand-rolling it here would be a second
//! source of truth for the same addresses.
//!
//! A number from this fixture is comparable to *this board's own history*, not
//! to a Cortex-M board's: a different ISA means a different instruction mix per
//! simulated step.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::main;

#[main]
fn main() -> ! {
    let mut acc: u32 = 0;
    loop {
        // `black_box` keeps LLVM from folding the loop away; the body stays a
        // fixed, branch-predictable add/compare/branch triple, same as the
        // Cortex-M and RISC-V fixtures'.
        acc = acc.wrapping_add(1);
        core::hint::black_box(acc);
    }
}
