#![no_std]
#![no_main]

// NUCLEO-G031K8: send a marker on the ST-LINK VCP and toggle the active-high
// LD3 user LED on PC6 (default MB1455 solder-bridge configuration).

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_CR: *mut u32 = 0x4002_1000 as *mut u32;
const RCC_IOPENR: *mut u32 = 0x4002_1034 as *mut u32;
const RCC_APBENR1: *mut u32 = 0x4002_103C as *mut u32;
const RCC_CR_HSION: u32 = 1 << 8;
const RCC_CR_HSIRDY: u32 = 1 << 10;
const IOPENR_GPIOAEN: u32 = 1 << 0;
const IOPENR_GPIOCEN: u32 = 1 << 2;
const APBENR1_USART2EN: u32 = 1 << 17;

const GPIOA_MODER: *mut u32 = 0x5000_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x5000_0020 as *mut u32;
const GPIOC_BASE: u32 = 0x5000_0800;
const GPIOC_MODER: *mut u32 = GPIOC_BASE as *mut u32;
const GPIOC_BSRR: *mut u32 = (GPIOC_BASE + 0x18) as *mut u32;
const LED_PIN: u32 = 6;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE_TE: u32 = (1 << 0) | (1 << 3);
const PCLK_HZ: u32 = 16_000_000;
const BAUD: u32 = 115_200;

#[entry]
fn main() -> ! {
    unsafe {
        // HSI16 is the G031 reset clock. The smoke leaves the clock tree at
        // its reset configuration and uses that rate for USART2.
        write_volatile(RCC_CR, read_volatile(RCC_CR) | RCC_CR_HSION);
        while read_volatile(RCC_CR) & RCC_CR_HSIRDY == 0 {}

        write_volatile(
            RCC_IOPENR,
            read_volatile(RCC_IOPENR) | IOPENR_GPIOAEN | IOPENR_GPIOCEN,
        );
        write_volatile(RCC_APBENR1, read_volatile(RCC_APBENR1) | APBENR1_USART2EN);

        // Configure PA2/PA3 for USART2 AF1, the board's default VCP route.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << 4) | (0b11 << 6))) | (0b10 << 4) | (0b10 << 6),
        );
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(
            GPIOA_AFRL,
            (afrl & !((0xF << 8) | (0xF << 12))) | (1 << 8) | (1 << 12),
        );

        // LD3 is active high on PC6 with the default SB12/SB13 settings.
        let moder = read_volatile(GPIOC_MODER);
        write_volatile(
            GPIOC_MODER,
            (moder & !(0b11 << (LED_PIN * 2))) | (0b01 << (LED_PIN * 2)),
        );

        write_volatile(USART2_BRR, PCLK_HZ / BAUD);
        write_volatile(USART2_CR1, USART_CR1_UE_TE);
    }

    puts(b"NUCLEO_G031K8_OK LED=PC6 VCP=USART2\n");

    for _ in 0..8 {
        led_set(true);
        spin(256);
        led_set(false);
        spin(256);
    }

    loop {
        cortex_m::asm::nop();
    }
}

fn led_set(on: bool) {
    let value = if on {
        1 << LED_PIN
    } else {
        1 << (LED_PIN + 16)
    };
    unsafe { write_volatile(GPIOC_BSRR, value) };
}

fn puts(text: &[u8]) {
    for &byte in text {
        unsafe {
            while read_volatile(USART2_ISR) & USART_ISR_TXE == 0 {}
            write_volatile(USART2_TDR, byte as u32);
        }
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
