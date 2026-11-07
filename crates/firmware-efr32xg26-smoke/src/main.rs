#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

// EFR32xG26 USART0 uses CMU_CLKEN0.USART0EN. The register map and clock bit
// follow Renode's efr32xg26.repl and the Series-2 profile used in each board
// descriptor.
const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN0_USART0: u32 = 1 << 9;

const USART0_BASE: usize = 0x400A_0000;
const USART0_EN: *mut u32 = (USART0_BASE + 0x04) as *mut u32;
const USART0_CMD: *mut u32 = (USART0_BASE + 0x14) as *mut u32;
const USART0_STATUS: *const u32 = (USART0_BASE + 0x18) as *const u32;
const USART0_CLKDIV: *mut u32 = (USART0_BASE + 0x1C) as *mut u32;
const USART0_TXDATA: *mut u32 = (USART0_BASE + 0x38) as *mut u32;

const USART_CMD_TXEN: u32 = 1 << 2;
const USART_STATUS_TXBL: u32 = 1 << 6;
const USART0_CLKDIV_115200: u32 = 2384;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(USART0_STATUS) & USART_STATUS_TXBL == 0 {}
        write_volatile(USART0_TXDATA, byte as u32);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(CMU_CLKEN0, CMU_CLKEN0_USART0);
        write_volatile(USART0_EN, 1);
        write_volatile(USART0_CLKDIV, USART0_CLKDIV_115200);
        write_volatile(USART0_CMD, USART_CMD_TXEN);
    }

    for byte in b"EFR32xG26 Cortex-M33 USART0 smoke\r\n" {
        putc(*byte);
    }

    loop {
        core::hint::spin_loop();
    }
}
