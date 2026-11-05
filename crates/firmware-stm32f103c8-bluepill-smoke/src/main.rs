// STM32F103C8T6 Blue Pill onboard LED smoke (Cortex-M3, thumbv7m-none-eabi).
// The board's PC13 LED is active-low; the smoke also checks GPIOC clock gating.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;
const GPIOC_CRH: *mut u32 = 0x4001_1004 as *mut u32;
const GPIOC_BSRR: *mut u32 = 0x4001_1010 as *mut u32;
const LED_PIN: u32 = 13;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOC (RCC_APB2ENR.IOPCEN), then set PC13 to 2 MHz push-pull.
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | (1 << 4));
        let mut crh = read_volatile(GPIOC_CRH);
        crh = (crh & !(0xF << 20)) | (0x2 << 20);
        write_volatile(GPIOC_CRH, crh);
        // Start with the active-low LED off.
        write_volatile(GPIOC_BSRR, 1 << LED_PIN);
    }

    loop {
        led_set(true);
        delay(20_000);
        led_set(false);
        delay(20_000);
    }
}

fn led_set(on: bool) {
    // A low PC13 sinks current through the board LED and lights it.
    let bits = if on {
        1 << (LED_PIN + 16)
    } else {
        1 << LED_PIN
    };
    unsafe { write_volatile(GPIOC_BSRR, bits) };
}

fn delay(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}
