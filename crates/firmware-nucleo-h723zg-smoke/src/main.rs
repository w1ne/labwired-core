// Bare-metal NUCLEO-H723ZG smoke: USART3 through ST-LINK VCP and LD1 on PB0.
// Register addresses and board wiring follow ST RM0468 and UM2407.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x5802_4400;
const RCC_AHB4ENR: *mut u32 = (RCC_BASE + 0xE0) as *mut u32;
const RCC_APB1LENR: *mut u32 = (RCC_BASE + 0xE8) as *mut u32;

const GPIOB_BASE: u32 = 0x5802_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_BSRR: *mut u32 = (GPIOB_BASE + 0x18) as *mut u32;

const GPIOD_BASE: u32 = 0x5802_0C00;
const GPIOD_MODER: *mut u32 = GPIOD_BASE as *mut u32;
const GPIOD_AFRH: *mut u32 = (GPIOD_BASE + 0x24) as *mut u32;

const USART3_BASE: u32 = 0x4000_4800;
const USART3_BRR: *mut u32 = (USART3_BASE + 0x0C) as *mut u32;
const USART3_ISR: *const u32 = (USART3_BASE + 0x1C) as *const u32;
const USART3_TDR: *mut u8 = (USART3_BASE + 0x28) as *mut u8;
const USART3_CR1: *mut u32 = USART3_BASE as *mut u32;

const TXE: u32 = 1 << 7;
const LED_PB0: u32 = 0;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB/GPIOD and USART3 on the H7 RCC buses.
        write_volatile(
            RCC_AHB4ENR,
            read_volatile(RCC_AHB4ENR) | (1 << 1) | (1 << 3),
        );
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 18));

        // LD1: PB0, general-purpose output, push-pull.
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(
            GPIOB_MODER,
            (moder & !(0b11 << (LED_PB0 * 2))) | (0b01 << (LED_PB0 * 2)),
        );

        // USART3 TX/RX: PD8/PD9, alternate function 7.
        let moder = read_volatile(GPIOD_MODER);
        let pins = (0b11 << (8 * 2)) | (0b11 << (9 * 2));
        write_volatile(
            GPIOD_MODER,
            (moder & !pins) | (0b10 << (8 * 2)) | (0b10 << (9 * 2)),
        );
        let af = read_volatile(GPIOD_AFRH);
        write_volatile(GPIOD_AFRH, (af & !0xFF) | 0x77);

        // Reset HSI is 64 MHz. BRR = 64 MHz / 115200, rounded to 556.
        write_volatile(USART3_BRR, 556);
        write_volatile(USART3_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }

    send(b"NUCLEO-H723ZG: USART3 + LD1\n");
    loop {
        led_set(true);
        delay(300);
        led_set(false);
        delay(300);
    }
}

fn led_set(on: bool) {
    let value = if on {
        1 << LED_PB0
    } else {
        1 << (LED_PB0 + 16)
    };
    unsafe { write_volatile(GPIOB_BSRR, value) };
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            let mut remaining = 20_000;
            while read_volatile(USART3_ISR) & TXE == 0 && remaining > 0 {
                remaining -= 1;
            }
            write_volatile(USART3_TDR, *byte);
        }
    }
}

fn delay(count: u32) {
    for _ in 0..count {
        core::hint::spin_loop();
    }
}
