#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;

const RCC_IOPENR: *mut u32 = 0x4002_1034 as *mut u32;
const RCC_APBENR1: *mut u32 = 0x4002_103C as *mut u32;

const GPIOA_MODER: *mut u32 = 0x5000_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x5000_0020 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x5000_0018 as *mut u32;

const USART2_CR1: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_440C as *mut u32;
const USART2_ISR: *const u32 = 0x4000_441C as *const u32;
const USART2_TDR: *mut u32 = 0x4000_4428 as *mut u32;

const LED_PIN: u32 = 5;
const UART_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA and USART2. The board leaves the core on HSI16 at reset.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | 1);
        write_volatile(RCC_APBENR1, read_volatile(RCC_APBENR1) | (1 << 17));

        // PA2/PA3 use AF1 for USART2; PA5 is LD4, configured as GPIO output.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << 4) | (0b11 << 6) | (0b11 << (LED_PIN * 2))))
                | (0b10 << 4)
                | (0b10 << 6)
                | (0b01 << (LED_PIN * 2)),
        );
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(
            GPIOA_AFRL,
            (afrl & !((0xF << 8) | (0xF << 12))) | (1 << 8) | (1 << 12),
        );

        // USART2 is clocked from reset HSI16. BRR 139 is the 115200 setting.
        write_volatile(USART2_BRR, 139);
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }

    for byte in b"NUCLEO_G071RB_OK\n" {
        putc(*byte);
    }

    loop {
        unsafe {
            write_volatile(GPIOA_BSRR, 1 << LED_PIN);
            write_volatile(GPIOA_BSRR, 1 << (LED_PIN + 16));
        }
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART2_ISR) & UART_TXE == 0 {}
        write_volatile(USART2_TDR, byte as u32);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
