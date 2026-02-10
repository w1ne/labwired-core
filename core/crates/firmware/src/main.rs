#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

use cortex_m_rt::entry;
use panic_halt as _;

#[entry]
fn main() -> ! {
    // UART1 Base Address from stm32f103.yaml: 0x40013800
    // DR (Data Register) offset: 0x04
    const UART1_BASE: u32 = 0x40013800;
    const UART1_DR: *mut u32 = (UART1_BASE + 0x04) as *mut u32;

    let message = b"Hello, LabWired! E2E Debugging Works!\n";

    loop {
        for &byte in message {
            unsafe {
                // Write byte to Data Register
                core::ptr::write_volatile(UART1_DR, byte as u32);
            }
            // Simple delay to prevent flooding
            for _ in 0..100 {
                cortex_m::asm::nop();
            }
        }
    }
}
