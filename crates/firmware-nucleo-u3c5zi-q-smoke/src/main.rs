//! Boot smoke for the ST NUCLEO-U3C5ZI-Q (Cortex-M33,
//! thumbv8m.main-none-eabi). It sends a marker through USART1 (board VCP on
//! PA9/PA10) and toggles LD1 on PA5.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RM0487 U3 RCC non-secure alias. MSIS is selected at 12 MHz after reset.
const RCC_BASE: u32 = 0x4602_0C00;
const RCC_AHB2ENR1: *mut u32 = (RCC_BASE + 0x8C) as *mut u32;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0xA4) as *mut u32;

const GPIOA_BASE: u32 = 0x4202_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRH: *mut u32 = (GPIOA_BASE + 0x24) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;
const GPIOA_ODR: *const u32 = (GPIOA_BASE + 0x14) as *const u32;
const LED_MASK: u32 = 1 << 5;

const USART1_BASE: u32 = 0x4001_3800;
const USART1_CR1: *mut u32 = USART1_BASE as *mut u32;
const USART1_BRR: *mut u32 = (USART1_BASE + 0x0C) as *mut u32;
const USART1_ISR: *const u32 = (USART1_BASE + 0x1C) as *const u32;
const USART1_TDR: *mut u32 = (USART1_BASE + 0x28) as *mut u32;
const USART_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // RCC_AHB2ENR1.GPIOAEN bit 0; RCC_APB2ENR.USART1EN bit 14.
        write_volatile(RCC_AHB2ENR1, read_volatile(RCC_AHB2ENR1) | 1);
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | (1 << 14));

        // PA5 is LD1. PA9/PA10 use AF7 for USART1 on the board's VCP route.
        let moder = read_volatile(GPIOA_MODER);
        let moder = (moder & !((0b11 << 10) | (0b11 << 18) | (0b11 << 20)))
            | (0b01 << 10)
            | (0b10 << 18)
            | (0b10 << 20);
        write_volatile(GPIOA_MODER, moder);
        let afrh = read_volatile(GPIOA_AFRH);
        write_volatile(
            GPIOA_AFRH,
            (afrh & !((0xF << 4) | (0xF << 8))) | (7 << 4) | (7 << 8),
        );

        // Rounded 12 MHz / 115200 baud divider for the reset MSIS clock.
        write_volatile(USART1_BRR, 104);
        write_volatile(USART1_CR1, (1 << 0) | (1 << 3)); // UE | TE
        write_volatile(GPIOA_BSRR, LED_MASK << 16);
        send(b"U3C5_USART1_OK\r\n");

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
            while read_volatile(USART1_ISR) & USART_TXE == 0 {}
            write_volatile(USART1_TDR, *byte as u32);
        }
    }
}

fn delay(iterations: u32) {
    for i in 0..iterations {
        core::hint::black_box(i);
    }
}
