#![no_std]
#![no_main]

//! STM32F429I-DISC1 smoke: boot the Cortex-M4 and toggle green user LED LD3 on PG13.
//! LD3 is active high; the smoke records PG13 transitions in the local model.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const GPIOG_BASE: u32 = 0x4002_1800;
const GPIOG_MODER: *mut u32 = GPIOG_BASE as *mut u32;
const GPIOG_BSRR: *mut u32 = (GPIOG_BASE + 0x18) as *mut u32;
const GPIOG_CLOCK_ENABLE: u32 = 1 << 6;
const LD3_PG13: u32 = 1 << 13;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOG and configure PG13 as an output. Start LD3 off before
        // the acceptance sequence; ST's BSP uses a high output to turn it on.
        write_volatile(RCC_AHB1ENR, read_volatile(RCC_AHB1ENR) | GPIOG_CLOCK_ENABLE);
        write_volatile(GPIOG_BSRR, LD3_PG13 << 16);
        let moder = read_volatile(GPIOG_MODER);
        write_volatile(GPIOG_MODER, (moder & !(0b11 << 26)) | (0b01 << 26));

        // Alternate the green user LED on and off for the GPIO edge test.
        for edge in 0..32 {
            write_volatile(
                GPIOG_BSRR,
                if edge & 1 == 0 {
                    LD3_PG13
                } else {
                    LD3_PG13 << 16
                },
            );
            core::hint::spin_loop();
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
