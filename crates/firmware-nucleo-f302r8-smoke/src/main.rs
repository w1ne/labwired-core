// NUCLEO-F302R8 smoke: USART2 VCP output and LD2 GPIO transitions.
// Reset HSI remains at 8 MHz; the firmware uses only the RCC, GPIO, and
// USART2 paths represented in stm32f302r8-nucleo-smoke.yaml.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_101C as *mut u32;

const GPIOA_BASE: u32 = 0x4800_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const LED_LD2_PA5: u32 = 1 << 5;
const USART_ISR_TXE: u32 = 1 << 7;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART2_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(USART2_TDR, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for &byte in text {
        putc(byte);
    }
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA (AHBENR.IOPAEN) and USART2 (APB1ENR.USART2EN).
        write_volatile(RCC_AHBENR, read_volatile(RCC_AHBENR) | (1 << 17));
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));

        // USART2_TX is AF7 on PA2, routed to the board's default ST-LINK VCP.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 4)) | (0b10 << 4));
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(GPIOA_AFRL, (afrl & !(0xF << 8)) | (7 << 8));

        // LD2 is active-high on PA5. Configure it as a general-purpose output.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));

        // Reset HSI is 8 MHz; divisor 69 is the nearest integer for 115200 baud.
        write_volatile(USART2_BRR, 69);
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }

    puts(b"NUCLEO_F302R8_OK\n");

    loop {
        unsafe { write_volatile(GPIOA_BSRR, LED_LD2_PA5) };
        spin(64);
        unsafe { write_volatile(GPIOA_BSRR, LED_LD2_PA5 << 16) };
        spin(64);
    }
}
