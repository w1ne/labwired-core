//! USART2 VCP + user LED smoke for ST NUCLEO-L432KC (Cortex-M4).
//!
//! UM1956 maps the board's default ST-LINK VCP to USART2 TX on PA2 and its
//! active-high user LED LD3 to PB3. The profile intentionally exercises only
//! those board-facing paths.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m::asm::nop;
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR1: *mut u32 = 0x4002_1058 as *mut u32;

const GPIOA_BASE: u32 = 0x4800_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;

const GPIOB_BASE: u32 = 0x4800_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_BSRR: *mut u32 = (GPIOB_BASE + 0x18) as *mut u32;
const LED_MASK: u32 = 1 << 3;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const ISR_TXE_TXFNF: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        clocks_init();
        gpio_init();
        usart2_init();
        usart2_write(b"L432KC_USART2_OK\n");
    }

    // Eight active-high LD3 pulses give the smoke a board-level GPIO oracle.
    for _ in 0..8 {
        unsafe { led_set(true) };
        delay(1_000);
        unsafe { led_set(false) };
        delay(1_000);
    }

    loop {
        nop();
    }
}

unsafe fn clocks_init() {
    // GPIOA/GPIOB on AHB2 and USART2 on APB1.
    write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 0b11);
    write_volatile(RCC_APB1ENR1, read_volatile(RCC_APB1ENR1) | (1 << 17));
}

unsafe fn gpio_init() {
    // PA2 = USART2_TX, alternate function 7 (RM0394); PB3 = active-high LD3.
    let pa2_mode = 0b11 << 4;
    write_volatile(
        GPIOA_MODER,
        (read_volatile(GPIOA_MODER) & !pa2_mode) | (0b10 << 4),
    );
    write_volatile(
        GPIOA_AFRL,
        (read_volatile(GPIOA_AFRL) & !(0xF << 8)) | (0x7 << 8),
    );

    let pb3_mode = 0b11 << 6;
    write_volatile(
        GPIOB_MODER,
        (read_volatile(GPIOB_MODER) & !pb3_mode) | (0b01 << 6),
    );
    led_set(false);
}

unsafe fn usart2_init() {
    // Reset MSI is 4 MHz: BRR 35 gives approximately 115200 baud.
    write_volatile(USART2_BRR, 35);
    write_volatile(USART2_CR1, (1 << 3) | 1); // TE | UE
}

unsafe fn usart2_write(bytes: &[u8]) {
    for byte in bytes {
        for _ in 0..10_000 {
            if read_volatile(USART2_ISR) & ISR_TXE_TXFNF != 0 {
                write_volatile(USART2_TDR, u32::from(*byte));
                break;
            }
        }
    }
}

unsafe fn led_set(on: bool) {
    write_volatile(GPIOB_BSRR, if on { LED_MASK } else { LED_MASK << 16 });
}

fn delay(iterations: u32) {
    for _ in 0..iterations {
        nop();
    }
}
