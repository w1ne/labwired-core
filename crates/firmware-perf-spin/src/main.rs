// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Throughput fixture for `scripts/perf/board_perf.py`.
//!
//! A bare ALU spin loop and nothing else: no peripheral is touched, so every
//! host instruction the simulator spends on a step is *its own* overhead —
//! decode, dispatch, the machine-advance orchestration, the per-cycle
//! peripheral walk and the NVIC scan. That is exactly the cost the perf gate
//! is meant to hold still.
//!
//! Built for `thumbv6m-none-eabi` so one ELF runs on every Cortex-M board in
//! the matrix (M0+ through M33) with an identical instruction mix — a per-board
//! number is then comparable to every other board's, not just to its own past.
//!
//! It is linked once per *memory map*, not once: `build.rs` takes the flash and
//! RAM origins from the environment, so the same spin loop also covers the
//! chips that put flash at 0x00000000 (nRF, Kinetis) or 0x10000000 (RP2xxx).
//! The linked image claims only 32K flash / 8K RAM so it fits the smallest
//! board in the matrix; a larger claim would put the reset stack pointer past
//! the end of modelled RAM and fault before `main`.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[entry]
fn main() -> ! {
    let mut acc: u32 = 0;
    loop {
        // `black_box` keeps LLVM from folding the loop away; the body stays a
        // fixed, branch-predictable add/compare/branch triple on every board.
        acc = acc.wrapping_add(1);
        core::hint::black_box(acc);
    }
}
