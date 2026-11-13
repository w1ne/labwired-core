// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM4S16C (1 MiB / 128 KiB, 100-pin): the SAM4S family smoke in `firmware-sam4s-smoke/src/family.rs`.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-sam4s-smoke/src/family.rs"]
mod family;

#[entry]
fn main() -> ! {
    family::run(&family::Part {
        name: "SAM4S16C",
        flash_bytes: 1024 * 1024,
        has_pioc: true,
        check_usart0_irq: true,
        check_usart1_irq: true,
        summary: 0x2001_FF00,
        board: None,
    })
}
