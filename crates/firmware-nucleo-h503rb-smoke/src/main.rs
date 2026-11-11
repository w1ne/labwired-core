//! GPIO + UART smoke for ST NUCLEO-H503RB (Cortex-M33,
//! thumbv8m.main-none-eabi). Blinks LD2 on PA5 and prints through the default
//! STLINK-V3EC VCP, USART3 on PA3/PA4 per UM3121 Table 13.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4402_0C8C as *mut u32;
const RCC_APB1LENR: *mut u32 = 0x4402_0C9C as *mut u32;
const GPIOA_BASE: u32 = 0x4202_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const GPIOA_ODR: *const u32 = (GPIOA_BASE + 0x14) as *const u32;
const LED_MASK: u32 = 1 << 5;

const USART3_BASE: u32 = 0x4000_4800;
const USART3_CR1: *mut u32 = USART3_BASE as *mut u32;
const USART3_BRR: *mut u32 = (USART3_BASE + 0x0C) as *mut u32;
const USART3_ISR: *const u32 = (USART3_BASE + 0x1C) as *const u32;
const USART3_TDR: *mut u32 = (USART3_BASE + 0x28) as *mut u32;
const USART_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // H5 RCC AHB2ENR.GPIOAEN (RM0492); UM3121 wires LD2 to PA5.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 1);
        // H5 RCC APB1LENR.USART3EN (bit 18); PA3/PA4 use AF7 for USART3.
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 18));

        let moder = read_volatile(GPIOA_MODER);
        let moder = (moder & !((0b11 << 6) | (0b11 << 8) | (0b11 << 10)))
            | (0b10 << 6)
            | (0b10 << 8)
            | (0b01 << 10);
        write_volatile(GPIOA_MODER, moder);
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(
            GPIOA_AFRL,
            (afrl & !((0xF << 12) | (0xF << 16))) | (7 << 12) | (7 << 16),
        );

        // The H5 reset clock is 64 MHz; BRR=556 gives approximately 115200 baud.
        write_volatile(USART3_BRR, 556);
        write_volatile(USART3_CR1, (1 << 0) | (1 << 3)); // UE | TE
        write_volatile(GPIOA_BSRR, LED_MASK << 16);
        send(b"H503_USART3_OK\r\n");

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

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            while read_volatile(USART3_ISR) & USART_TXE == 0 {}
            write_volatile(USART3_TDR, *byte as u32);
        }
    }
}

fn delay(iterations: u32) {
    for i in 0..iterations {
        core::hint::black_box(i);
    }
}
