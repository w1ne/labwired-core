//! GPIO + UART smoke for ST NUCLEO-H533RE (Cortex-M33,
//! thumbv8m.main-none-eabi). Blinks LD2 on PA5 and writes a marker through
//! USART2, which UM3121 maps to the board's default STLINK-V3EC VCP on PA2/PA3.

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

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;
const USART_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // H5 RCC AHB2ENR GPIOAEN (RM0481); UM3121 wires LD2 to PA5.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 1);
        // USART2EN is APB1LENR bit 17; PA2/PA3 use AF7 for USART2.
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 17));
        let moder = read_volatile(GPIOA_MODER);
        let moder = (moder & !((0b11 << 4) | (0b11 << 6) | (0b11 << 10)))
            | (0b10 << 4)
            | (0b10 << 6)
            | (0b01 << 10);
        write_volatile(GPIOA_MODER, moder);
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(
            GPIOA_AFRL,
            (afrl & !((0xF << 8) | (0xF << 12))) | (7 << 8) | (7 << 12),
        );
        // H533 reset HSI is 64 MHz; BRR=556 gives 115200 baud (RM0481).
        write_volatile(USART2_BRR, 556);
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE
        write_volatile(GPIOA_BSRR, LED_MASK << 16);

        send(b"H533_USART2_OK\r\n");

        // Read back each output level in addition to the smoke's PA5 edge
        // oracle, so a stalled or incorrectly configured output fails early.
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
            while read_volatile(USART2_ISR) & USART_TXE == 0 {}
            write_volatile(USART2_TDR, *byte as u32);
        }
    }
}

fn delay(iterations: u32) {
    for i in 0..iterations {
        core::hint::black_box(i);
    }
}
