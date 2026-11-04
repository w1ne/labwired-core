#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB4ENR: *mut u32 = 0x5802_44E0 as *mut u32;
const GPIOB: u32 = 0x5802_0400;
const GPIO_MODER: *mut u32 = GPIOB as *mut u32;
const GPIO_ODR: *const u32 = (GPIOB + 0x14) as *const u32;
const GPIO_BSRR: *mut u32 = (GPIOB + 0x18) as *mut u32;
const LD1_PB0: u32 = 1;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB on the H7 reset-clock tree, then configure PB0 as output.
        write_volatile(RCC_AHB4ENR, 1 << 1);
        let moder = read_volatile(GPIO_MODER);
        write_volatile(GPIO_MODER, (moder & !0b11) | 0b01);

        // Start with LD1 off. UM2408 specifies that the user LEDs are active high.
        write_volatile(GPIO_BSRR, LD1_PB0 << 16);
    }

    // Toggle the board's real LD1/PB0 path repeatedly for the smoke oracle.
    for _ in 0..32 {
        unsafe {
            let high = read_volatile(GPIO_ODR) & LD1_PB0 == 0;
            write_volatile(
                GPIO_BSRR,
                if high { LD1_PB0 } else { LD1_PB0 << 16 },
            );
        }
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}
