#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// Matches `SystemBus::new()` default `uart1` base.
const UART_TX_PTR: *mut u8 = 0x4000_C000 as *mut u8;

#[entry]
fn main() -> ! {
    let mut count: u32 = 0;

    // Initial sync
    unsafe {
        core::ptr::write_volatile(UART_TX_PTR, b'S');
        core::ptr::write_volatile(UART_TX_PTR, b'T');
        core::ptr::write_volatile(UART_TX_PTR, b'A');
        core::ptr::write_volatile(UART_TX_PTR, b'R');
        core::ptr::write_volatile(UART_TX_PTR, b'T');
        core::ptr::write_volatile(UART_TX_PTR, b'\n');
    }

    loop {
        // High-density instruction loop
        for i in 0..1_000_000 {
            count = count.wrapping_add(i);
            core::hint::black_box(count);
        }

        // Occasional UART pulse
        unsafe {
            core::ptr::write_volatile(UART_TX_PTR, b'.');
        }
    }
}
