#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m::asm::nop;
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4002_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4002_0418 as *mut u32;
const LD1_PB0: u32 = 1;

#[entry]
fn main() -> ! {
    unsafe {
        // RM0033: enable GPIOB on AHB1 before configuring PB0.
        write_volatile(RCC_AHB1ENR, read_volatile(RCC_AHB1ENR) | (1 << 1));

        // Configure PB0 as a general-purpose output for the default LD1 route.
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !0b11) | 0b01);

        // UM1974: LD1 is active high on PB0 with the default solder bridges.
        for _ in 0..8 {
            write_volatile(GPIOB_BSRR, LD1_PB0);
            for _ in 0..16 {
                nop();
            }
            write_volatile(GPIOB_BSRR, LD1_PB0 << 16);
            for _ in 0..16 {
                nop();
            }
        }
    }

    loop {
        nop();
    }
}
