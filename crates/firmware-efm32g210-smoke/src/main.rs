#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// EFM32G USART0 register map: CMD@0x0C, STATUS@0x10, TXDATA@0x34.
// The imported system smoke asserts only TX buffer readiness and captured bytes.
const USART0_CMD: *mut u32 = 0x4000_C00C as *mut u32;
const USART0_STATUS: *const u32 = 0x4000_C010 as *const u32;
const USART0_TXDATA: *mut u32 = 0x4000_C034 as *mut u32;

const CMD_TXEN: u32 = 1 << 0;
const STATUS_TXBL: u32 = 1 << 6;

fn putc(byte: u8) {
    unsafe {
        let mut remaining = 100_000u32;
        while read_volatile(USART0_STATUS) & STATUS_TXBL == 0 && remaining > 0 {
            remaining -= 1;
        }
        write_volatile(USART0_TXDATA, byte as u32);
    }
}

#[entry]
fn main() -> ! {
    unsafe { write_volatile(USART0_CMD, CMD_TXEN) };
    for byte in b"EFM32G210 USART0 smoke\r\n" {
        putc(*byte);
    }
    loop {
        core::hint::spin_loop();
    }
}
