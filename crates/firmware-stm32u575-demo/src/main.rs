#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

// NUCLEO-U575ZI-Q VCP maps COM1 to USART1 (PA9/PA10) in stm32u5xx_nucleo.c.
// TDR offset 0x28 per RM0456 (USART v2 register map, same as WBA52/H563).
const USART1_TDR_PTR: *mut u8 = (0x4001_3800 + 0x28) as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(USART1_TDR_PTR, b'O');
        core::ptr::write_volatile(USART1_TDR_PTR, b'K');
        core::ptr::write_volatile(USART1_TDR_PTR, b'\n');
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
