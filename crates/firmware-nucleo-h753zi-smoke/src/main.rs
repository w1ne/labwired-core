//! USART3 + LD1 smoke for ST NUCLEO-H753ZI (Cortex-M7, thumbv7em-none-eabi).
//! UM2407 maps the default ST-LINK VCP to USART3 PD8/PD9 and LD1 to PB0.

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
const GPIOB_ODR: *const u32 = (GPIOB_BASE + 0x14) as *const u32;
const LED_MASK: u32 = 1;

const GPIOD_BASE: u32 = 0x5802_0C00;
const GPIOD_MODER: *mut u32 = GPIOD_BASE as *mut u32;
const GPIOD_AFRH: *mut u32 = (GPIOD_BASE + 0x24) as *mut u32;

const USART3_BASE: u32 = 0x4000_4800;
const USART3_CR1: *mut u32 = USART3_BASE as *mut u32;
const USART3_BRR: *mut u32 = (USART3_BASE + 0x0C) as *mut u32;
const USART3_ISR: *const u32 = (USART3_BASE + 0x1C) as *const u32;
const USART3_TDR: *mut u32 = (USART3_BASE + 0x28) as *mut u32;
const USART_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // RM0433 AHB4ENR: GPIOB, GPIOC, and GPIOD; APB1LENR: USART3.
        write_volatile(RCC_AHB4ENR, (1 << 1) | (1 << 2) | (1 << 3));
        write_volatile(RCC_APB1LENR, 1 << 18);

        // LD1 is active-high PB0. USART3 uses AF7 on PD8 (TX) and PD9 (RX).
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !0b11) | 0b01);
        let moder = read_volatile(GPIOD_MODER);
        write_volatile(
            GPIOD_MODER,
            (moder & !((0b11 << 16) | (0b11 << 18))) | (0b10 << 16) | (0b10 << 18),
        );
        let afr = read_volatile(GPIOD_AFRH);
        write_volatile(GPIOD_AFRH, (afr & !0xFF) | 0x77);

        // The reset clock is 64 MHz HSI. 556 selects approximately 115200 baud.
        write_volatile(USART3_BRR, 556);
        write_volatile(USART3_CR1, (1 << 0) | (1 << 3)); // UE | TE
        write_volatile(GPIOB_BSRR, LED_MASK << 16);
        send(b"H753_USART3_OK\n");

        loop {
            write_volatile(GPIOB_BSRR, LED_MASK);
            if read_volatile(GPIOB_ODR) & LED_MASK == 0 {
                loop {
                    core::hint::spin_loop();
                }
            }
            delay(2_000);

            write_volatile(GPIOB_BSRR, LED_MASK << 16);
            if read_volatile(GPIOB_ODR) & LED_MASK != 0 {
                loop {
                    core::hint::spin_loop();
                }
            }
            delay(2_000);
        }
    }
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            while read_volatile(USART3_ISR) & USART_TXE == 0 {}
            write_volatile(USART3_TDR, u32::from(*byte));
        }
    }
}

fn delay(iterations: u32) {
    for _ in 0..iterations {
        cortex_m::asm::nop();
    }
}
