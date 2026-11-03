#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_IOPENR: *mut u32 = 0x4002_1034 as *mut u32;
const RCC_APBENR1: *mut u32 = 0x4002_103C as *mut u32;

const GPIOB_MODER: *mut u32 = 0x5000_0400 as *mut u32;
const GPIOB_AFRH: *mut u32 = 0x5000_0424 as *mut u32;

const USART3_CR1: *mut u32 = 0x4000_4800 as *mut u32;
const USART3_BRR: *mut u32 = 0x4000_480C as *mut u32;
const USART3_ISR: *const u32 = 0x4000_481C as *const u32;
const USART3_TDR: *mut u32 = 0x4000_4828 as *mut u32;

const USART_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB and USART3. The firmware uses the reset HSI16 clock.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | (1 << 1));
        write_volatile(RCC_APBENR1, read_volatile(RCC_APBENR1) | (1 << 18));

        // The board VCP's selected USART3 route uses PB8 TX and PB9 RX (AF0).
        // AFRH resets to AF0; select alternate-function mode on both pads.
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(
            GPIOB_MODER,
            (moder & !((0b11 << 16) | (0b11 << 18)))
                | (0b10 << 16)
                | (0b10 << 18),
        );
        let afrh = read_volatile(GPIOB_AFRH);
        write_volatile(GPIOB_AFRH, afrh & !0xFF);

        // HSI16 / 115200 = 139. Configure USART3 8N1 transmit.
        write_volatile(USART3_BRR, 139);
        write_volatile(USART3_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }

    for byte in b"STM32G071B_DISCO_OK\r\n" {
        unsafe {
            while read_volatile(USART3_ISR) & USART_TXE == 0 {}
            write_volatile(USART3_TDR, *byte as u32);
        }
    }

    loop {}
}
