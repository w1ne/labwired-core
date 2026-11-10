//! GPIO smoke for ST NUCLEO-G491RE (Cortex-M4, thumbv7em-none-eabi).
//! Drives LD2 on PA5 through the STM32G4 GPIO register interface.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const GPIOA_BASE: u32 = 0x4800_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const GPIOA_BRR: *mut u32 = (GPIOA_BASE + 0x28) as *mut u32;
const GPIOA_ODR: *const u32 = (GPIOA_BASE + 0x14) as *const u32;
const LED_MASK: u32 = 1 << 5;

#[entry]
fn main() -> ! {
    unsafe {
        // G4 RCC AHB2ENR.IOPAEN (RM0440); LD2 is wired to PA5 by UM2505.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 1);
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));
        write_volatile(GPIOA_BRR, LED_MASK);

        // Read back each level as the board LED is driven. The smoke oracle
        // also records PA5 transitions, so a print or an idle register value
        // cannot stand in for the GPIO behavior.
        loop {
            write_volatile(GPIOA_BSRR, LED_MASK);
            if read_volatile(GPIOA_ODR) & LED_MASK == 0 {
                loop {
                    core::hint::spin_loop();
                }
            }
            delay(1000);

            write_volatile(GPIOA_BRR, LED_MASK);
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
