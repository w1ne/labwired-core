#![no_std]
#![no_main]

//! NUCLEO-G0B1RE board smoke: USART2 marker on the ST-LINK VCP and LD4 on PA5.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_IOPENR: *mut u32 = 0x4002_1034 as *mut u32;
const RCC_APBENR1: *mut u32 = 0x4002_103C as *mut u32;
const GPIOA_BASE: u32 = 0x5000_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const PA5: u32 = 1 << 5;
const USART_TXE: u32 = 1 << 7;
const USART_UE_TE: u32 = (1 << 0) | (1 << 3);
const TX_SPIN_LIMIT: u32 = 100_000;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA (IOPENR bit 0) and USART2 (APBENR1 bit 17) at reset HSI.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | 1);
        write_volatile(RCC_APBENR1, read_volatile(RCC_APBENR1) | (1 << 17));

        // PA5 = output for the board's active-high LD4; PA2 = USART2_TX AF1.
        let moder = read_volatile(GPIOA_MODER);
        let moder = (moder & !((0b11 << 10) | (0b11 << 4))) | (0b01 << 10) | (0b10 << 4);
        write_volatile(GPIOA_MODER, moder);
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(GPIOA_AFRL, (afrl & !(0xF << 8)) | (1 << 8));

        // 16 MHz reset clock / 115200 baud rounds to BRR=139.
        write_volatile(USART2_BRR, 139);
        write_volatile(USART2_CR1, USART_UE_TE);
        write_volatile(GPIOA_BSRR, PA5 << 16);
    }

    send(b"G0B1_USART2_OK LD4=PA5\r\n");

    loop {
        unsafe {
            write_volatile(GPIOA_BSRR, PA5);
        }
        settle_led_edge();
        unsafe { write_volatile(GPIOA_BSRR, PA5 << 16) };
        settle_led_edge();
    }
}

fn settle_led_edge() {
    for _ in 0..2048 {
        core::hint::spin_loop();
    }
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        let mut ready = false;
        for _ in 0..TX_SPIN_LIMIT {
            if unsafe { read_volatile(USART2_ISR) } & USART_TXE != 0 {
                ready = true;
                break;
            }
        }
        if !ready {
            return;
        }
        unsafe { write_volatile(USART2_TDR, *byte as u32) };
    }
}
