//! STM32H735 demo on the embassy-stm32 async HAL — a SECOND, independent
//! bring-up path from stm32h7xx-hal. `embassy_stm32::init()` runs its own
//! RCC/PWR/FLASH configuration + peripheral-clock enables, so booting this
//! exercises the H7 model differently. Markers go out raw over USART3.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use panic_halt as _;

const USART3: u32 = 0x4000_4800;
fn raw_puts(s: &[u8]) {
    for &b in s {
        for _ in 0..10_000 {
            if unsafe { core::ptr::read_volatile((USART3 + 0x1C) as *const u32) } & (1 << 7) != 0 {
                break;
            }
        }
        unsafe { core::ptr::write_volatile((USART3 + 0x28) as *mut u8, b) };
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    raw_puts(b"EMBASSY: entry\n");

    // Full embassy RCC/PWR/FLASH bring-up + peripheral-clock enables.
    let p = embassy_stm32::init(Default::default());
    raw_puts(b"EMBASSY: init ok (clocks/pwr/flash up)\n");

    let mut led = Output::new(p.PA5, Level::High, Speed::Low);
    raw_puts(b"EMBASSY: gpio ok\n");
    raw_puts(b"EMBASSY: DONE\n");

    let mut n = 0u32;
    loop {
        led.toggle();
        n = n.wrapping_add(1);
        if n <= 8 {
            raw_puts(b"EMBASSY: tick\n");
        }
        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
