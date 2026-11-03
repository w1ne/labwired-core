#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4602_0C8C as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4602_0CA4 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4202_0000 as *mut u32;
const GPIOA_AFRH: *mut u32 = 0x4202_0024 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4202_0400 as *mut u32;
const GPIOB_AFRH: *mut u32 = 0x4202_0424 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4202_0418 as *mut u32;
const USART1_CR1: *mut u32 = 0x4001_3800 as *mut u32;
const USART1_BRR: *mut u32 = 0x4001_380C as *mut u32;
const USART1_ISR: *const u32 = 0x4001_381C as *const u32;
const USART1_TDR: *mut u32 = 0x4001_3828 as *mut u32;

const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_TE: u32 = 1 << 3;
const LED_LD1_PB4: u32 = 1 << 4;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART1_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(USART1_TDR, byte as u32);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA/GPIOB for USART1's PA8/PB12 pads and LD1 on PB4.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 0b11);
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | (1 << 14));

        // USART1 VCP1 uses PA8 RX and PB12 TX, both AF7 (ST UM3301).
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 16)) | (0b10 << 16));
        let afrh = read_volatile(GPIOA_AFRH);
        write_volatile(GPIOA_AFRH, (afrh & !0xF) | 7);

        let moder = read_volatile(GPIOB_MODER);
        let moder = (moder & !((0b11 << 8) | (0b11 << 24))) | (0b01 << 8) | (0b10 << 24);
        write_volatile(GPIOB_MODER, moder);
        let afrh = read_volatile(GPIOB_AFRH);
        write_volatile(GPIOB_AFRH, (afrh & !(0xF << 16)) | (7 << 16));

        // LD1 is active low. Start illuminated, then toggle it below.
        write_volatile(GPIOB_BSRR, LED_LD1_PB4 << 16);

        // The WBA reset clock is 16 MHz; BRR 139 gives approximately 115200 baud.
        write_volatile(USART1_BRR, 139);
        write_volatile(USART1_CR1, USART_CR1_UE | USART_CR1_TE);
    }

    for byte in b"NUCLEO_WBA55CG_OK\n" {
        putc(*byte);
    }

    let mut iterations = 0u32;
    let mut led_high = false;
    loop {
        iterations += 1;
        if iterations == 64 {
            iterations = 0;
            led_high = !led_high;
            unsafe {
                write_volatile(
                    GPIOB_BSRR,
                    if led_high {
                        LED_LD1_PB4
                    } else {
                        LED_LD1_PB4 << 16
                    },
                );
            }
        }
    }
}
