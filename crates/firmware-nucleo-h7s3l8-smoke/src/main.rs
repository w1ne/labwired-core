#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC: u32 = 0x5802_4400;
const RCC_AHB4ENR: *mut u32 = (RCC + 0xE0) as *mut u32;

const GPIOD: u32 = 0x5802_0C00;
const MODER: u32 = 0x00;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;

const LD1_PD10: u32 = 1 << 10;

fn reg_rmw(addr: u32, clear: u32, set: u32) {
    unsafe {
        let register = addr as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOD for the NUCLEO-H7S3L8 user LED.
        write_volatile(RCC_AHB4ENR, 1 << 3);

        // LD1 is active-high on PD10.
        reg_rmw(GPIOD + MODER, 0b11 << 20, 0b01 << 20);
        write_volatile((GPIOD + BSRR) as *mut u32, LD1_PD10 << 16);
    }

    // Make LD1 transitions observable as real GPIO output events.
    for _ in 0..32 {
        unsafe {
            let high = read_volatile((GPIOD + ODR) as *const u32) & LD1_PD10 == 0;
            write_volatile(
                (GPIOD + BSRR) as *mut u32,
                if high { LD1_PD10 } else { LD1_PD10 << 16 },
            );
        }
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}
