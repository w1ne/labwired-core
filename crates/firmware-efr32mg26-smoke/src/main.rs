#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

// EFR32MG26 Series-2 console path: CMU_CLKEN2.USART1EN and USART1's
// Series-2 register layout. Addresses and bit positions follow the existing
// silicon-grounded EFR32MG26 profile in configs/chips/efr32mg26.yaml.
const CMU_CLKEN2: *mut u32 = 0x4000_806C as *mut u32;
const CMU_CLKEN2_USART1: u32 = 1 << 7;

const USART1_BASE: usize = 0x400A_4000;
const USART1_EN: *mut u32 = (USART1_BASE + 0x04) as *mut u32;
const USART1_CMD: *mut u32 = (USART1_BASE + 0x14) as *mut u32;
const USART1_STATUS: *const u32 = (USART1_BASE + 0x18) as *const u32;
const USART1_CLKDIV: *mut u32 = (USART1_BASE + 0x1C) as *mut u32;
const USART1_TXDATA: *mut u32 = (USART1_BASE + 0x38) as *mut u32;

const USART_CMD_TXEN: u32 = 1 << 2;
const USART_STATUS_TXBL: u32 = 1 << 6;
const USART1_CLKDIV_115200: u32 = 2384;

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(USART1_STATUS) & USART_STATUS_TXBL == 0 {}
        write_volatile(USART1_TXDATA, byte as u32);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(CMU_CLKEN2, CMU_CLKEN2_USART1);
        write_volatile(USART1_EN, 1);
        write_volatile(USART1_CLKDIV, USART1_CLKDIV_115200);
        write_volatile(USART1_CMD, USART_CMD_TXEN);
    }

    for byte in b"EFR32MG26 Cortex-M33 USART1 smoke\r\n" {
        putc(*byte);
    }

    loop {
        core::hint::spin_loop();
    }
}
