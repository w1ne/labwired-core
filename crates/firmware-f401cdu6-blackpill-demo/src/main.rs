#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

// Simulation smoke uses UART2 so it exercises the same minimal F401 UART path.
const USART2_DR_PTR: *mut u8 = (0x4000_4400 + 0x04) as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(USART2_DR_PTR, b'O');
        core::ptr::write_volatile(USART2_DR_PTR, b'K');
        core::ptr::write_volatile(USART2_DR_PTR, b'\n');
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
