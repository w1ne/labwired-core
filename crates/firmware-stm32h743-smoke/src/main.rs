#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB4ENR: *mut u32 = 0x5802_44E0 as *mut u32;
const RCC_APB1LENR: *mut u32 = 0x5802_44E8 as *mut u32;
const GPIOB: u32 = 0x5802_0400;
const GPIOD: u32 = 0x5802_0C00;
const USART3: u32 = 0x4000_4800;
const MODER: u32 = 0x00;
const AFRH: u32 = 0x24;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;
const USART_CR1: *mut u32 = USART3 as *mut u32;
const USART_BRR: *mut u32 = (USART3 + 0x0C) as *mut u32;
const USART_ISR: *const u32 = (USART3 + 0x1C) as *const u32;
const USART_TDR: *mut u32 = (USART3 + 0x28) as *mut u32;
const TXE: u32 = 1 << 7;
const LED: u32 = 1;

fn register_rmw(address: u32, clear: u32, set: u32) {
    unsafe {
        let register = address as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART_ISR) & TXE == 0 {}
        write_volatile(USART_TDR, byte as u32);
    }
}

fn send(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn set_led(on: bool) {
    let bit = if on { LED } else { LED << 16 };
    unsafe { write_volatile((GPIOB + BSRR) as *mut u32, bit) };
}

fn led_is_on() -> bool {
    unsafe { read_volatile((GPIOB + ODR) as *const u32) & LED != 0 }
}

fn delay() {
    for _ in 0..200_000 {
        unsafe {
            let _ = read_volatile((GPIOB + ODR) as *const u32);
        }
    }
}

#[entry]
fn main() -> ! {
    // Enable GPIOB/GPIOD and USART3 in the STM32H7 RCC.
    unsafe {
        write_volatile(
            RCC_AHB4ENR,
            read_volatile(RCC_AHB4ENR) | (1 << 1) | (1 << 3),
        );
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 18));
    }

    // Route PD8 to USART3_TX (AF7), configure USART3, and make PB0 an output.
    register_rmw(GPIOD + MODER, 0b11 << 16, 0b10 << 16);
    register_rmw(GPIOD + AFRH, 0xF << 0, 7);
    register_rmw(GPIOB + MODER, 0b11, 0b01);
    unsafe {
        write_volatile(USART_BRR, 555); // 64 MHz / 115200
        write_volatile(USART_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }

    set_led(false);
    send(b"STM32H7 register smoke\r\n");
    loop {
        let on = !led_is_on();
        set_led(on);
        send(if on {
            b"gpiob0=high\r\n"
        } else {
            b"gpiob0=low\r\n"
        });
        delay();
    }
}
