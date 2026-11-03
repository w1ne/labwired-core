#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC: u32 = 0x4002_3800;
const RCC_AHB1ENR: *mut u32 = (RCC + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC + 0x40) as *mut u32;

const GPIOA: u32 = 0x4002_0000;
const GPIOD: u32 = 0x4002_0C00;
const USART2: u32 = 0x4000_4400;

const MODER: u32 = 0x00;
const AFRL: u32 = 0x20;
const BSRR: u32 = 0x18;
const USART_SR: *const u32 = USART2 as *const u32;
const USART_DR: *mut u32 = (USART2 + 0x04) as *mut u32;
const USART_BRR: *mut u32 = (USART2 + 0x08) as *mut u32;
const USART_CR1: *mut u32 = (USART2 + 0x0C) as *mut u32;

const TXE: u32 = 1 << 7;
const LED: u32 = 1 << 12;

fn register_rmw(address: u32, clear: u32, set: u32) {
    unsafe {
        let register = address as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

fn configure_usart2_pins() {
    for pin in [2u32, 3] {
        register_rmw(GPIOA + MODER, 0b11 << (pin * 2), 0b10 << (pin * 2));
        register_rmw(GPIOA + AFRL, 0xF << (pin * 4), 7 << (pin * 4));
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART_SR) & TXE == 0 {}
        write_volatile(USART_DR, byte as u32);
    }
}

fn send(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn set_led(on: bool) {
    let bit = if on { LED } else { LED << 16 };
    unsafe { write_volatile((GPIOD + BSRR) as *mut u32, bit) };
}

fn delay() {
    for _ in 0..40_000 {
        unsafe {
            let _ = read_volatile((GPIOD + 0x14) as *const u32);
        }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA/GPIOD on AHB1 and USART2 on APB1.
        write_volatile(
            RCC_AHB1ENR,
            read_volatile(RCC_AHB1ENR) | (1 << 0) | (1 << 3),
        );
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));
    }

    configure_usart2_pins();
    register_rmw(GPIOD + MODER, 0b11 << 24, 0b01 << 24);
    unsafe {
        // The smoke checks the register path, not baud accuracy or a clock tree.
        write_volatile(USART_BRR, 0x8B);
        write_volatile(USART_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }

    set_led(false);
    send(b"STM32F4 shared profile smoke\n");

    loop {
        set_led(true);
        delay();
        set_led(false);
        delay();
    }
}
