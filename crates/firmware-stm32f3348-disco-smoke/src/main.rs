#![no_std]
#![no_main]

//! STM32F3348-DISCO smoke: boot the Cortex-M4 and toggle red user LED LD3 on PB6.
//! LD3 is active high; the smoke records PB6 transitions in the local model.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const GPIOB_BASE: u32 = 0x4800_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_BSRR: *mut u32 = (GPIOB_BASE + 0x18) as *mut u32;
const GPIOB_CLOCK_ENABLE: u32 = 1 << 18;
const LD3_PB6: u32 = 1 << 6;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB and configure PB6 as an output. Set it low first so the
        // active-high red LED starts off before the acceptance sequence.
        write_volatile(RCC_AHBENR, read_volatile(RCC_AHBENR) | GPIOB_CLOCK_ENABLE);
        write_volatile(GPIOB_BSRR, LD3_PB6 << 16);
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !(0b11 << 12)) | (0b01 << 12));

        // Alternate LED on (PB6 high) and off (PB6 low) for the GPIO edge test.
        for edge in 0..32 {
            write_volatile(
                GPIOB_BSRR,
                if edge & 1 == 0 {
                    LD3_PB6
                } else {
                    LD3_PB6 << 16
                },
            );
            core::hint::spin_loop();
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
