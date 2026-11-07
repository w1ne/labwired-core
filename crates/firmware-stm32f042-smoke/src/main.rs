#![no_std]
#![no_main]

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// Register addresses and offsets follow the vendor-only STM32F042x SVD
// register map. The smoke checks the engine model at these locations.
const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4800_0018 as *mut u32;
const USART1_CR1: *mut u32 = 0x4001_3800 as *mut u32;
const USART1_BRR: *mut u32 = 0x4001_380C as *mut u32;
const USART1_ISR: *const u32 = 0x4001_381C as *const u32;
const USART1_TDR: *mut u32 = 0x4001_3828 as *mut u32;

const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_TE: u32 = 1 << 3;
const LED_PIN: u32 = 5;

fn putc(byte: u8) {
    unsafe {
        let mut guard = 0u32;
        while read_volatile(USART1_ISR) & USART_ISR_TXE == 0 && guard < 100_000 {
            guard += 1;
        }
        write_volatile(USART1_TDR, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn set_led(on: bool) {
    let value = if on {
        1 << LED_PIN
    } else {
        1 << (LED_PIN + 16)
    };
    unsafe { write_volatile(GPIOA_BSRR, value) };
}

#[inline(never)]
fn delay() {
    for _ in 0..10_000 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA and USART1 using the modeled F0 RCC register layout.
        write_volatile(RCC_AHBENR, read_volatile(RCC_AHBENR) | (1 << 17));
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | (1 << 14));

        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !(0b11 << (LED_PIN * 2))) | (0b01 << (LED_PIN * 2)),
        );

        // The smoke checks UART register behavior and captured bytes, not baud timing.
        write_volatile(USART1_BRR, 0x45);
        write_volatile(USART1_CR1, USART_CR1_UE | USART_CR1_TE);
    }

    set_led(false);
    puts(b"STM32F042 profile smoke\r\n");
    loop {
        set_led(true);
        delay();
        set_led(false);
        delay();
    }
}
