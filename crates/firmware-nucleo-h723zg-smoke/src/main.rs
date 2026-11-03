#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC: u32 = 0x5802_4400;
const RCC_AHB4ENR: *mut u32 = (RCC + 0xE0) as *mut u32;
const RCC_APB1LENR: *mut u32 = (RCC + 0xE8) as *mut u32;

const GPIOB: u32 = 0x5802_0400;
const GPIOD: u32 = 0x5802_0C00;
const MODER: u32 = 0x00;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;
const AFRH: u32 = 0x24;

const USART3: u32 = 0x4000_4800;
const USART_CR1: *mut u32 = USART3 as *mut u32;
const USART_BRR: *mut u32 = (USART3 + 0x0C) as *mut u32;
const USART_ISR: *const u32 = (USART3 + 0x1C) as *const u32;
const USART_TDR: *mut u32 = (USART3 + 0x28) as *mut u32;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE_TE: u32 = (1 << 0) | (1 << 3);

const LD1_PB0: u32 = 1;

fn reg_rmw(addr: u32, clear: u32, set: u32) {
    unsafe {
        let register = addr as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(USART_TDR, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for &byte in text {
        putc(byte);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB/GPIOD and USART3 using the H7 reset-clock tree.
        write_volatile(RCC_AHB4ENR, (1 << 1) | (1 << 3));
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 18));

        // LD1 is active-high on PB0. Configure PD8 as USART3_TX (AF7).
        reg_rmw(GPIOB + MODER, 0b11, 0b01);
        reg_rmw(GPIOD + MODER, 0b11 << 16, 0b10 << 16);
        reg_rmw(GPIOD + AFRH, 0xF, 0x7);

        // The reset HSI is 64 MHz. BRR rounds 64 MHz / 115200 to 556.
        write_volatile(USART_BRR, 556);
        write_volatile(USART_CR1, USART_CR1_UE_TE);
        write_volatile((GPIOB + BSRR) as *mut u32, LD1_PB0 << 16);
    }

    puts(b"NUCLEO_H723ZG_OK LD1=PB0 USART3=PD8\n");

    // Make LD1 transitions observable as real GPIO output events.
    for _ in 0..32 {
        unsafe {
            let high = read_volatile((GPIOB + ODR) as *const u32) & LD1_PB0 == 0;
            write_volatile(
                (GPIOB + BSRR) as *mut u32,
                if high { LD1_PB0 } else { LD1_PB0 << 16 },
            );
        }
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}
