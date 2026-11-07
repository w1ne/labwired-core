#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// The imported profile maps the modeled EFM32 UART block at 0x4000_C400.
const USART1_CMD: *mut u32 = 0x4000_C40C as *mut u32;
const USART1_STATUS: *const u32 = 0x4000_C410 as *const u32;
const USART1_TXDATA: *mut u32 = 0x4000_C434 as *mut u32;

const CMD_TXEN: u32 = 1 << 0;
const STATUS_TXBL: u32 = 1 << 6;

fn putc(byte: u8) {
    unsafe {
        let mut remaining = 100_000u32;
        while read_volatile(USART1_STATUS) & STATUS_TXBL == 0 && remaining > 0 {
            remaining -= 1;
        }
        write_volatile(USART1_TXDATA, byte as u32);
    }
}

#[entry]
fn main() -> ! {
    unsafe { write_volatile(USART1_CMD, CMD_TXEN) };
    for byte in b"EFM32ZG222 USART1 smoke\r\n" {
        putc(*byte);
    }
    loop {
        core::hint::spin_loop();
    }
}
