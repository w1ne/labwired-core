#![no_std]
#![no_main]

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// STM32L0 register map (RM0377): RCC, IOPORT GPIOA and USART2.
const RCC_IOPENR: *mut u32 = 0x4002_102C as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_1038 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x5000_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x5000_0020 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x5000_0018 as *mut u32;
const USART2_CR1: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_440C as *mut u32;
const USART2_ISR: *const u32 = 0x4000_441C as *const u32;
const USART2_TDR: *mut u32 = 0x4000_4428 as *mut u32;

const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_TE: u32 = 1 << 3;
const LED_PIN: u32 = 5;

fn putc(byte: u8) {
    unsafe {
        let mut guard = 0u32;
        while read_volatile(USART2_ISR) & USART_ISR_TXE == 0 && guard < 100_000 {
            guard += 1;
        }
        write_volatile(USART2_TDR, byte as u32);
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
        // GPIOA uses RCC IOPENR on STM32L0; USART2 is on APB1.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | 1);
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));

        // PA2/PA3 are AF4 for USART2; PA5 is the output exercised by the smoke.
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
            (afrl & !((0xF << 8) | (0xF << 12))) | (4 << 8) | (4 << 12),
        );

        // The fixture checks register behavior and UART capture, not baud timing.
        write_volatile(USART2_BRR, 0x45);
        write_volatile(USART2_CR1, USART_CR1_UE | USART_CR1_TE);
    }

    set_led(false);
    puts(b"STM32L071 profile smoke\r\n");
    loop {
        set_led(true);
        delay();
        set_led(false);
        delay();
    }
}
