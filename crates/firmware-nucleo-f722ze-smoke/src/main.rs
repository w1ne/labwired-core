#![no_std]
#![no_main]

//! NUCLEO-F722ZE smoke: boot the Cortex-M7 and toggle green user LED LD1 on PB0.
//! The smoke observes GPIOB transitions in the local simulator test.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4002_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4002_0418 as *mut u32;
const GPIOB_CLOCK_ENABLE: u32 = 1 << 1;
const LD1_PB0: u32 = 1;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB and configure PB0 as an output. Start LD1 off before
        // the acceptance sequence; ST documents a high output as LED on.
        write_volatile(
            RCC_AHB1ENR,
            read_volatile(RCC_AHB1ENR) | GPIOB_CLOCK_ENABLE,
        );
        write_volatile(GPIOB_BSRR, LD1_PB0 << 16);
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !0b11) | 0b01);

        // Alternate LD1 on and off; the simulator test asserts these edges.
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

        // Leave LD1 on as a visible completion marker on the physical board.
        write_volatile(GPIOB_BSRR, LD1_PB0);
    }

    loop {
        core::hint::spin_loop();
    }
}
