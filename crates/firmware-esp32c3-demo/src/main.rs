#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

const UART_TX_PTR: *mut u8 = 0x6000_0000 as *mut u8;

#[entry]
fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(UART_TX_PTR, b'E');
        core::ptr::write_volatile(UART_TX_PTR, b'S');
        core::ptr::write_volatile(UART_TX_PTR, b'P');
        core::ptr::write_volatile(UART_TX_PTR, b' ');
        core::ptr::write_volatile(UART_TX_PTR, b'O');
        core::ptr::write_volatile(UART_TX_PTR, b'K');
        core::ptr::write_volatile(UART_TX_PTR, b'\n');
    }

    loop {}
}
