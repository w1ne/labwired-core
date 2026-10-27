// LabWired - NUCLEO-F103RB UART/LED smoke (Cortex-M3, thumbv7m-none-eabi).
//
// Uses the reset HSI clock (8 MHz), USART2 on PA2/PA3 for the Nucleo VCP,
// and LD2 on PA5. These are bare STM32F1 registers from RM0008.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_101C as *mut u32;

const GPIOA_CRL: *mut u32 = 0x4001_0800 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4001_0810 as *mut u32;

const USART2_SR: *const u32 = 0x4000_4400 as *const u32;
const USART2_DR: *mut u32 = 0x4000_4404 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_4408 as *mut u32;
const USART2_CR1: *mut u32 = 0x4000_440C as *mut u32;

const LED_PIN: u32 = 5;
const TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA (APB2ENR.IOPAEN) and USART2 (APB1ENR.USART2EN).
        write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | (1 << 2));
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));
        gpio_uart_init();
    }

    send(b"OK\r\n");

    loop {
        led_set(true);
        delay(20_000);
        led_set(false);
        delay(20_000);
    }
}

unsafe fn gpio_uart_init() {
    let mut crl = read_volatile(GPIOA_CRL);

    // PA2 = USART2_TX, alternate-function push-pull, 50 MHz (nibble 0xB).
    crl = (crl & !(0xF << 8)) | (0xB << 8);
    // PA3 = USART2_RX, floating input (nibble 0x4).
    crl = (crl & !(0xF << 12)) | (0x4 << 12);
    // PA5 = LD2, general-purpose push-pull output, 2 MHz (nibble 0x2).
    crl = (crl & !(0xF << 20)) | (0x2 << 20);
    write_volatile(GPIOA_CRL, crl);

    // HSI reset clock is 8 MHz; BRR = 8 MHz / 115200 ≈ 0x45.
    write_volatile(USART2_BRR, 0x45);
    // UE | TE: enable USART2 and its transmitter.
    write_volatile(USART2_CR1, (1 << 13) | (1 << 3));
}

fn led_set(on: bool) {
    let bits = if on {
        1 << LED_PIN
    } else {
        1 << (LED_PIN + 16)
    };
    unsafe { write_volatile(GPIOA_BSRR, bits) };
}

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

fn delay(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}
