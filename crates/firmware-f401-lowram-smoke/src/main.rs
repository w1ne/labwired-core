// LabWired - STM32F401 variant UART/LED smoke (Cortex-M4, thumbv7em-none-eabi).
//
// Uses the reset HSI clock (16 MHz), USART2 on PA2/PA3 (AF7) and PA5 as a
// GPIO output. These are bare STM32F4 registers from RM0368. Every helper is
// out of line, so each call pushes LR to the stack: an initial SP outside
// SRAM faults on the first call instead of passing silently.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;

const GPIOA_MODER: *mut u32 = 0x4002_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4002_0018 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4002_0020 as *mut u32;

const USART2_SR: *const u32 = 0x4000_4400 as *const u32;
const USART2_DR: *mut u32 = 0x4000_4404 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_4408 as *mut u32;
const USART2_CR1: *mut u32 = 0x4000_440C as *mut u32;

const LED_PIN: u32 = 5;
const TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA (AHB1ENR.GPIOAEN) and USART2 (APB1ENR.USART2EN).
        write_volatile(RCC_AHB1ENR, read_volatile(RCC_AHB1ENR) | 1);
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));
    }
    gpio_uart_init();

    send(b"OK\r\n");

    loop {
        led_set(true);
        delay(20_000);
        led_set(false);
        delay(20_000);
    }
}

#[inline(never)]
fn gpio_uart_init() {
    unsafe {
        // PA2/PA3 alternate function (0b10), PA5 general-purpose output (0b01).
        let mut moder = read_volatile(GPIOA_MODER);
        moder &= !((0b11 << 4) | (0b11 << 6) | (0b11 << 10));
        moder |= (0b10 << 4) | (0b10 << 6) | (0b01 << 10);
        write_volatile(GPIOA_MODER, moder);

        // AF7 = USART2 on PA2/PA3.
        let afrl = read_volatile(GPIOA_AFRL) & !((0xF << 8) | (0xF << 12));
        write_volatile(GPIOA_AFRL, afrl | (7 << 8) | (7 << 12));

        // HSI reset clock is 16 MHz; BRR = 16 MHz / 115200 ≈ 0x8B.
        write_volatile(USART2_BRR, 0x8B);
        // UE | TE: enable USART2 and its transmitter.
        write_volatile(USART2_CR1, (1 << 13) | (1 << 3));
    }
}

#[inline(never)]
fn led_set(on: bool) {
    let bits = if on {
        1 << LED_PIN
    } else {
        1 << (LED_PIN + 16)
    };
    unsafe { write_volatile(GPIOA_BSRR, bits) };
}

#[inline(never)]
fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            let mut remaining = 20_000;
            while read_volatile(USART2_SR) & TXE == 0 && remaining > 0 {
                remaining -= 1;
            }
            write_volatile(USART2_DR, *byte as u32);
        }
    }
}

#[inline(never)]
fn delay(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}
