#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const UART0_CMD: *mut u32 = 0x4000_E00C as *mut u32;
const UART0_STATUS: *const u32 = 0x4000_E010 as *const u32;
const UART0_TXDATA: *mut u32 = 0x4000_E034 as *mut u32;

const CMD_TXEN: u32 = 1 << 0;
const STATUS_TXBL: u32 = 1 << 6;

fn putc(byte: u8) {
    unsafe {
        let mut remaining = 100_000u32;
        while read_volatile(UART0_STATUS) & STATUS_TXBL == 0 && remaining > 0 {
            remaining -= 1;
        }
        write_volatile(UART0_TXDATA, byte as u32);
    }
}

#[entry]
fn main() -> ! {
    unsafe { write_volatile(UART0_CMD, CMD_TXEN) };
    for byte in b"EZR32 Wonder Gecko UART0 smoke\r\n" {
        putc(*byte);
    }
    loop {
        core::hint::spin_loop();
    }
}
