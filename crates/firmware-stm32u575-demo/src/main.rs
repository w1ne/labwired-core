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
// RCC_APB2ENR @ 0x4602_0C00 + 0xA4 (RM0456 §11.8.18), USART1EN bit 14.
// Real firmware enables the peripheral clock before its first register
// access; with the U5 `clock:` gate declared, an unclocked TDR write is
// dropped and no "OK" would reach the VCP.
const RCC_APB2ENR_PTR: *mut u32 = (0x4602_0C00 + 0xA4) as *mut u32;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(RCC_APB2ENR_PTR, 1 << 14);
        core::ptr::write_volatile(USART1_TDR_PTR, b'O');
        core::ptr::write_volatile(USART1_TDR_PTR, b'K');
        core::ptr::write_volatile(USART1_TDR_PTR, b'\n');
    }

    #[allow(clippy::empty_loop)] // bare-metal firmware: park the core forever
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    #[allow(clippy::empty_loop)] // bare-metal firmware: park the core forever
    loop {}
}
