#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use panic_halt as _;
use riscv_rt::entry;

// Matches `SystemBus::new()` default `uart1` base.
// Note: In a real system, we'd use a HAL, but for a minimal test this is fine.
const UART_TX_PTR: *mut u8 = 0x4000_C000 as *mut u8;

#[entry]
fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(UART_TX_PTR, b'R');
        core::ptr::write_volatile(UART_TX_PTR, b'V');
        core::ptr::write_volatile(UART_TX_PTR, b' ');
        core::ptr::write_volatile(UART_TX_PTR, b'O');
        core::ptr::write_volatile(UART_TX_PTR, b'K');
        core::ptr::write_volatile(UART_TX_PTR, b'\n');
    }

    // Deterministic "PC stuck" for `no_progress` tests.
    loop {}
}
