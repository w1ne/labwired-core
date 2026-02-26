#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;

// nRF52832 UART0 TXD register: base 0x40002000 + offset 0x51C
const UART0_TXD: *mut u8 = 0x4000_251C as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    unsafe {
        core::ptr::write_volatile(UART0_TXD, b'O');
        core::ptr::write_volatile(UART0_TXD, b'K');
        core::ptr::write_volatile(UART0_TXD, b'\n');
    }

    loop {}
}
