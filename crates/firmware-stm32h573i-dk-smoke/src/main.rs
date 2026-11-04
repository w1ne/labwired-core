#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4402_0C8C as *mut u32;
const GPIOI_MODER: *mut u32 = 0x4202_2000 as *mut u32;
const GPIOI_BSRR: *mut u32 = 0x4202_2018 as *mut u32;
const LD1_PI9: u32 = 1 << 9;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOI and configure PI9 for the board's active-low green LD1.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | (1 << 8));
        let moder = read_volatile(GPIOI_MODER);
        write_volatile(GPIOI_MODER, (moder & !(0b11 << 18)) | (0b01 << 18));
    }

    // Alternate LED on/off: on this board LD1 lights when PI9 is low.
    let mut pin_high = true;
    for _ in 0..32 {
        unsafe {
            write_volatile(GPIOI_BSRR, if pin_high { LD1_PI9 } else { LD1_PI9 << 16 });
        }
        pin_high = !pin_high;
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}
