// Bare-metal NUCLEO-F446RE smoke: USART2 through ST-LINK VCP and LD2 on PA5.
// Register addresses and board wiring follow ST RM0390 and UM1724.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_3800;
const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x40) as *mut u32;

const GPIOA_BASE: u32 = 0x4002_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR: *const u32 = USART2_BASE as *const u32;
const USART2_DR: *mut u32 = (USART2_BASE + 0x04) as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x08) as *mut u32;
const USART2_CR1: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;

const TXE: u32 = 1 << 7;
const LED_PA5: u32 = 5;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA on AHB1 and USART2 on APB1.
        write_volatile(RCC_AHB1ENR, read_volatile(RCC_AHB1ENR) | 1);
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));

        // LD2: PA5 general-purpose output. USART2 TX/RX: PA2/PA3 alternate
        // function. The PA13..PA15 debug pads keep their reset mode.
        let pins = (0b11 << (LED_PA5 * 2)) | (0b11 << (2 * 2)) | (0b11 << (3 * 2));
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !pins) | (0b01 << (LED_PA5 * 2)) | (0b10 << (2 * 2)) | (0b10 << (3 * 2)),
        );
        let af = read_volatile(GPIOA_AFRL);
        write_volatile(GPIOA_AFRL, (af & !0xFF00) | 0x7700);

        // Reset HSI is 16 MHz. BRR = 16 MHz / 115200, rounded to 139.
        write_volatile(USART2_BRR, 139);
        write_volatile(USART2_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }

    send(b"NUCLEO-F446RE: USART2 + LD2\n");
    loop {
        led_set(true);
        delay(300);
        led_set(false);
        delay(300);
    }
}

fn led_set(on: bool) {
    let value = if on {
        1 << LED_PA5
    } else {
        1 << (LED_PA5 + 16)
    };
    unsafe { write_volatile(GPIOA_BSRR, value) };
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            let mut remaining = 20_000;
            while read_volatile(USART2_SR) & TXE == 0 && remaining > 0 {
                remaining -= 1;
            }
            write_volatile(USART2_DR, u32::from(*byte));
        }
    }
}

fn delay(count: u32) {
    for _ in 0..count {
        core::hint::spin_loop();
    }
}
