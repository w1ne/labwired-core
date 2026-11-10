//! GPIO smoke for ST NUCLEO-H533RE (Cortex-M33, thumbv8m.main-none-eabi).
//! Blinks LD2 on PA5 through the H5 GPIO register interface.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4402_0C8C as *mut u32;
const GPIOA_BASE: u32 = 0x4202_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const GPIOA_ODR: *const u32 = (GPIOA_BASE + 0x14) as *const u32;
const LED_MASK: u32 = 1 << 5;

#[entry]
fn main() -> ! {
    unsafe {
        // H5 RCC AHB2ENR GPIOAEN (RM0481); UM3121 wires LD2 to PA5.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 1);
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));
        write_volatile(GPIOA_BSRR, LED_MASK << 16);

        // Read back each output level in addition to the smoke's PA5 edge
        // oracle, so a stalled or incorrectly configured output fails early.
        loop {
            write_volatile(GPIOA_BSRR, LED_MASK);
            if read_volatile(GPIOA_ODR) & LED_MASK == 0 {
                loop {
                    core::hint::spin_loop();
                }
            }
            delay(1000);

            write_volatile(GPIOA_BSRR, LED_MASK << 16);
            if read_volatile(GPIOA_ODR) & LED_MASK != 0 {
                loop {
                    core::hint::spin_loop();
                }
            }
            delay(1000);
        }
    }
}

fn delay(iterations: u32) {
    for i in 0..iterations {
        core::hint::black_box(i);
    }
}
