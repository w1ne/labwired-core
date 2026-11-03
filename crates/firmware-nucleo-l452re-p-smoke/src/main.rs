#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC (RM0394): AHB2ENR @ 0x4C, APB1ENR2 @ 0x5C.
const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR2: *mut u32 = 0x4002_105C as *mut u32;

// STM32 GPIO v2: GPIOA @ 0x48000000 and GPIOB @ 0x48000400.
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4800_0020 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4800_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4800_0418 as *mut u32;

// LPUART1 (USART v2 register layout): CR1, BRR, ISR and TDR.
const LPUART1_CR1: *mut u32 = 0x4000_8000 as *mut u32;
const LPUART1_BRR: *mut u32 = 0x4000_800C as *mut u32;
const LPUART1_ISR: *const u32 = 0x4000_801C as *const u32;
const LPUART1_TDR: *mut u32 = 0x4000_8028 as *mut u32;

const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_TE: u32 = 1 << 3;
const LED_LD4_PB13: u32 = 1 << 13;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(LPUART1_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(LPUART1_TDR, byte as u32);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable the GPIOA/GPIOB and LPUART1 clocks before touching them.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 0b11);
        write_volatile(RCC_APB1ENR2, read_volatile(RCC_APB1ENR2) | 1);

        // PA2/PA3 are the default LPUART1 VCP pins, alternate function 8.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << 4) | (0b11 << 6))) | (0b10 << 4) | (0b10 << 6),
        );
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(
            GPIOA_AFRL,
            (afrl & !((0xF << 8) | (0xF << 12))) | (8 << 8) | (8 << 12),
        );

        // LD4 is PB13, active high. Set it as a push-pull output and start low.
        let moder = read_volatile(GPIOB_MODER);
        write_volatile(GPIOB_MODER, (moder & !(0b11 << 26)) | (0b01 << 26));
        write_volatile(GPIOB_BSRR, LED_LD4_PB13 << 16);

        // Reset MSI is 4 MHz. LPUART BRR = 256 * 4 MHz / 115200 = 8889.
        write_volatile(LPUART1_BRR, 8889);
        write_volatile(LPUART1_CR1, USART_CR1_UE | USART_CR1_TE);
    }

    for byte in b"NUCLEO_L452RE_P_OK\n" {
        putc(*byte);
    }

    let mut led_high = false;
    loop {
        led_high = !led_high;
        unsafe {
            write_volatile(
                GPIOB_BSRR,
                if led_high {
                    LED_LD4_PB13
                } else {
                    LED_LD4_PB13 << 16
                },
            );
        }
    }
}
