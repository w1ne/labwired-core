#![no_std]
#![no_main]

//! NUCLEO-H755ZI-Q M7 smoke: toggle active-high green user LED LD1 on PB0.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB4ENR: *mut u32 = 0x5802_44E0 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x5802_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x5802_0418 as *mut u32;
const LD1_PB0: u32 = 1;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB and configure PB0 as an output for LD1.
        write_volatile(RCC_AHB4ENR, read_volatile(RCC_AHB4ENR) | (1 << 1));
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !0b11) | 0b01);

        // Produce alternating high/low GPIO writes for the LD1 smoke oracle.
        for edge in 0..32 {
            write_volatile(
                GPIOB_BSRR,
                if edge & 1 == 0 {
                    LD1_PB0
                } else {
                    LD1_PB0 << 16
                },
            );
            core::hint::spin_loop();
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
