#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4602_0C8C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4202_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4202_0018 as *mut u32;
const LD1_PA7: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA and configure PA7 as a general-purpose output.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 1);
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 14)) | (0b01 << 14));
    }

    let mut led_high = false;
    let mut iterations = 0u32;
    loop {
        iterations += 1;
        if iterations == 64 {
            iterations = 0;
            led_high = !led_high;
            unsafe {
                // LD1 is active low; alternate the PA7 electrical level.
                write_volatile(GPIOA_BSRR, if led_high { LD1_PA7 } else { LD1_PA7 << 16 });
            }
        }
    }
}
