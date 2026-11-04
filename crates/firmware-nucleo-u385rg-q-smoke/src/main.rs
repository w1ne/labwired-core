#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR1: *mut u32 = 0x4003_0C8C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4202_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4202_0018 as *mut u32;
const LD2_PA5: u32 = 1 << 5;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA and configure PA5 for the board's active-high green LD2.
        write_volatile(RCC_AHB2ENR1, read_volatile(RCC_AHB2ENR1) | 1);
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));
    }

    let mut high = false;
    for _ in 0..32 {
        unsafe {
            write_volatile(GPIOA_BSRR, if high { LD2_PA5 } else { LD2_PA5 << 16 });
        }
        high = !high;
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}
