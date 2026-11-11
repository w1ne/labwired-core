//! GPIO + USART2 smoke for ST NUCLEO-F446RE (Cortex-M4, thumbv7em-none-eabi).
//! The board's LD2 is PA5 and its default ST-LINK VCP uses USART2 on PA2/PA3.

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
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const GPIOA_ODR: *const u32 = (GPIOA_BASE + 0x14) as *const u32;
const LED_MASK: u32 = 1 << 5;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR: *const u32 = USART2_BASE as *const u32;
const USART2_DR: *mut u32 = (USART2_BASE + 0x04) as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x08) as *mut u32;
const USART2_CR1: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA and USART2 clocks (RM0390 RCC AHB1ENR/APB1ENR).
        write_volatile(RCC_AHB1ENR, read_volatile(RCC_AHB1ENR) | 1);
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));

        // PA2/PA3 use AF7 for USART2; PA5 is the active-high LD2 output.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << 4) | (0b11 << 6) | (0b11 << 10)))
                | (0b10 << 4)
                | (0b10 << 6)
                | (0b01 << 10),
        );
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(
            GPIOA_AFRL,
            (afrl & !((0xF << 8) | (0xF << 12))) | (7 << 8) | (7 << 12),
        );

        // Reset HSI is 16 MHz. BRR=139 selects approximately 115200 baud.
        write_volatile(USART2_BRR, 139);
        write_volatile(USART2_CR1, (1 << 13) | (1 << 3)); // UE | TE
        write_volatile(GPIOA_BSRR, LED_MASK << 16);
        send(b"F446RE_USART2_OK\n");

        loop {
            write_volatile(GPIOA_BSRR, LED_MASK);
            if read_volatile(GPIOA_ODR) & LED_MASK == 0 {
                loop {
                    core::hint::spin_loop();
                }
            }
            delay(2_000);

            write_volatile(GPIOA_BSRR, LED_MASK << 16);
            if read_volatile(GPIOA_ODR) & LED_MASK != 0 {
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
            while read_volatile(USART2_SR) & USART_TXE == 0 {}
            write_volatile(USART2_DR, u32::from(*byte));
        }
    }
}

fn delay(iterations: u32) {
    for _ in 0..iterations {
        cortex_m::asm::nop();
    }
}
