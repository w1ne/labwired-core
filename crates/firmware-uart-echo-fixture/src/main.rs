#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

// Matches `SystemBus::new()` default `uart1` base, Stm32F1 register layout:
// SR (status) @ +0x00, DR (data, shared TX/RX) @ +0x04. RXNE is bit 5 of SR.
const UART_SR_PTR: *const u8 = 0x4000_C000 as *const u8;
const UART_DR_PTR: *mut u8 = 0x4000_C004 as *mut u8;
const SR_RXNE: u8 = 1 << 5;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        // Signal readiness before the loop so the assertion side can tell a
        // pre-boot injection race from a genuinely-dropped byte.
        core::ptr::write_volatile(UART_DR_PTR, b'R');
        core::ptr::write_volatile(UART_DR_PTR, b'E');
        core::ptr::write_volatile(UART_DR_PTR, b'A');
        core::ptr::write_volatile(UART_DR_PTR, b'D');
        core::ptr::write_volatile(UART_DR_PTR, b'Y');
        core::ptr::write_volatile(UART_DR_PTR, b'\n');

        loop {
            let sr = core::ptr::read_volatile(UART_SR_PTR);
            if sr & SR_RXNE != 0 {
                let byte = core::ptr::read_volatile(UART_DR_PTR);
                if byte == b'Q' {
                    // Command byte: reply with a fixed marker instead of an echo.
                    core::ptr::write_volatile(UART_DR_PTR, b'B');
                    core::ptr::write_volatile(UART_DR_PTR, b'Y');
                    core::ptr::write_volatile(UART_DR_PTR, b'E');
                    core::ptr::write_volatile(UART_DR_PTR, b'\n');
                } else {
                    // Plain echo.
                    core::ptr::write_volatile(UART_DR_PTR, byte);
                }
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
