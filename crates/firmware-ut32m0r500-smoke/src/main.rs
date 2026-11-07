#![no_std]
#![no_main]

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// Renode's UT32M0R500 platform uses Gaisler APBUART0 at 0x40004000.
// APBUART DATA is at +0x00; STATUS has TS|TE ready bits at +0x04.
const UART0_DATA: *mut u32 = 0x4000_4000 as *mut u32;
const UART0_STATUS: *const u32 = 0x4000_4004 as *const u32;
const UART_TX_READY: u32 = 0x06;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UART0_STATUS) & UART_TX_READY != UART_TX_READY {}
        write_volatile(UART0_DATA, u32::from(byte));
    }
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

#[entry]
fn main() -> ! {
    puts(b"UT32M0R500 Cortex-M0+ UART0 smoke\n");

    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
