#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

// Matches `SystemBus::new()` default `uart1` base.
const UART_TX_PTR: *mut u8 = 0x4000_C000 as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(UART_TX_PTR, b'O');
        core::ptr::write_volatile(UART_TX_PTR, b'K');
        core::ptr::write_volatile(UART_TX_PTR, b'\n');
    }

    // Deterministic "PC stuck" for `no_progress` tests.
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
