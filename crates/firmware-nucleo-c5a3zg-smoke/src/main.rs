#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4402_0C8C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4202_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4202_0018 as *mut u32;
const LD1_PA5: u32 = 1 << 5;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA, then configure PA5 for the board's active-high LD1.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 1);
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));
    }

    let mut high = false;
    for _ in 0..32 {
        unsafe {
            write_volatile(GPIOA_BSRR, if high { LD1_PA5 } else { LD1_PA5 << 16 });
        }
        high = !high;
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}
