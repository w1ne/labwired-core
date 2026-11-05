#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const UART0: u32 = 0x4000_C000;
const UART_DR: *mut u32 = UART0 as *mut u32;
const UART_FR: *const u32 = (UART0 + 0x018) as *const u32;
const UART_IBRD: *mut u32 = (UART0 + 0x024) as *mut u32;
const UART_FBRD: *mut u32 = (UART0 + 0x028) as *mut u32;
const UART_LCRH: *mut u32 = (UART0 + 0x02C) as *mut u32;
const UART_CTL: *mut u32 = (UART0 + 0x030) as *mut u32;
const FR_TXFF: u32 = 1 << 5;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UART_FR) & FR_TXFF != 0 {}
        write_volatile(UART_DR, u32::from(byte));
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(UART_CTL, 0);
        // CC2538 reset clock 16 MHz, PL011-compatible 115200-baud divisors.
        write_volatile(UART_IBRD, 8);
        write_volatile(UART_FBRD, 44);
        write_volatile(UART_LCRH, 0x60);
        write_volatile(UART_CTL, (1 << 0) | (1 << 8) | (1 << 9));
    }

    for byte in b"ZOLERTIA FIREFLY CC2538 Cortex-M3 UART0 smoke\n" {
        putc(*byte);
    }

    loop {
        core::hint::spin_loop();
    }
}
