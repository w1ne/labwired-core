// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM4S family row (`sam4s`, SAM4S2C geometry): see `family.rs`.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

mod family;

#[entry]
fn main() -> ! {
    family::run(&family::Part {
        name: "SAM4S",
        flash_bytes: 128 * 1024,
        has_pioc: true,
        check_usart0_irq: true,
        summary: 0x2000_FF00,
        board: None,
    })
}
