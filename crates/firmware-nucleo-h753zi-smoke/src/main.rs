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
const GPIOE: u32 = 0x5802_1000;
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

const LD1_PB0: u32 = 1 << 0;
const LD2_PE1: u32 = 1 << 1;
const LD3_PB14: u32 = 1 << 14;

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

fn toggle_led(port: u32, mask: u32) {
    unsafe {
        let high = read_volatile((port + ODR) as *const u32) & mask == 0;
        write_volatile(
            (port + BSRR) as *mut u32,
            if high { mask } else { mask << 16 },
        );
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB/GPIOD/GPIOE and USART3 on the H7 reset-clock tree.
        write_volatile(RCC_AHB4ENR, (1 << 1) | (1 << 3) | (1 << 4));
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 18));

        // All three user LEDs are active-high. Route PD8 to USART3_TX (AF7).
        reg_rmw(
            GPIOB + MODER,
            (0b11 << 0) | (0b11 << 28),
            (0b01 << 0) | (0b01 << 28),
        );
        reg_rmw(GPIOE + MODER, 0b11 << 2, 0b01 << 2);
        reg_rmw(GPIOD + MODER, 0b11 << 16, 0b10 << 16);
        reg_rmw(GPIOD + AFRH, 0xF, 0x7);

        // The reset HSI is 64 MHz. BRR rounds 64 MHz / 115200 to 556.
        write_volatile(USART_BRR, 556);
        write_volatile(USART_CR1, USART_CR1_UE_TE);
        write_volatile((GPIOB + BSRR) as *mut u32, (LD1_PB0 | LD3_PB14) << 16);
        write_volatile((GPIOE + BSRR) as *mut u32, LD2_PE1 << 16);
    }

    puts(b"NUCLEO_H753ZI_OK LD1=PB0 LD2=PE1 LD3=PB14 USART3=PD8\n");

    // Repeatedly toggle the three physical user LED pads so each GPIO path
    // produces observable edges in the onboarding oracle.
    for _ in 0..32 {
        toggle_led(GPIOB, LD1_PB0);
        toggle_led(GPIOE, LD2_PE1);
        toggle_led(GPIOB, LD3_PB14);
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}
