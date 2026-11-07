#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// Offsets follow the MAX32650 UART layout implemented by LabWired's UART model.
const UART0_BASE: usize = 0x4004_2000;
const UART0_STATUS: *const u32 = (UART0_BASE + 0x08) as *const u32;
const UART0_FIFO: *mut u32 = (UART0_BASE + 0x1C) as *mut u32;
const STATUS_TX_EMPTY: u32 = 1 << 6;

fn putc(byte: u8) {
    unsafe {
        let mut remaining = 100_000u32;
        while read_volatile(UART0_STATUS) & STATUS_TX_EMPTY == 0 && remaining > 0 {
            remaining -= 1;
        }
        write_volatile(UART0_FIFO, byte as u32);
    }
}

#[entry]
fn main() -> ! {
    for byte in b"MAX32652 UART0 smoke\r\n" {
        putc(*byte);
    }
    loop {
        core::hint::spin_loop();
    }
}
