#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2350 IO_BANK0 and SIO registers. GP0 is Pico 2 W header pin 1.
const GPIO0_CTRL: *mut u32 = 0x4002_8004 as *mut u32;
const SIO_GPIO_OUT_SET: *mut u32 = 0xD000_0014 as *mut u32;
const SIO_GPIO_OUT_CLR: *mut u32 = 0xD000_0018 as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = 0xD000_0024 as *mut u32;

const GPIO_FUNC_SIO: u32 = 5;
const GPIO0_MASK: u32 = 1;

#[entry]
fn main() -> ! {
    unsafe {
        // Select the RP2350 SIO function for the board's exposed GP0 pad.
        core::ptr::write_volatile(GPIO0_CTRL, GPIO_FUNC_SIO);
        core::ptr::write_volatile(SIO_GPIO_OE_SET, GPIO0_MASK);
    }

    loop {
        unsafe { core::ptr::write_volatile(SIO_GPIO_OUT_SET, GPIO0_MASK) };
        delay(4_000);
        unsafe { core::ptr::write_volatile(SIO_GPIO_OUT_CLR, GPIO0_MASK) };
        delay(4_000);
    }
}

fn delay(n: u32) {
    for _ in 0..n {
        cortex_m::asm::nop();
    }
}
