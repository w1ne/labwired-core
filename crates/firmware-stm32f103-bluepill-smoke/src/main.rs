// Blue Pill STM32F103C8 smoke: USART1 on PA9 and the on-board active-low LED on PC13.
// It stays on the reset HSI clock (8 MHz), avoiding claims about PLL setup or USB.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;

const GPIOA_CRH: *mut u32 = 0x4001_0804 as *mut u32;
const GPIOC_CRH: *mut u32 = 0x4001_1004 as *mut u32;
const GPIOC_BSRR: *mut u32 = 0x4001_1010 as *mut u32;

const USART1_SR: *const u32 = 0x4001_3800 as *const u32;
const USART1_DR: *mut u32 = 0x4001_3804 as *mut u32;
const USART1_BRR: *mut u32 = 0x4001_3808 as *mut u32;
const USART1_CR1: *mut u32 = 0x4001_380C as *mut u32;

const LED_PIN: u32 = 13;
const TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA, GPIOC and USART1 on APB2 (RM0008, RCC_APB2ENR).
        write_volatile(
            RCC_APB2ENR,
            read_volatile(RCC_APB2ENR) | (1 << 2) | (1 << 4) | (1 << 14),
        );
        gpio_uart_init();
    }

    send(b"BLUEPILL_F103C8_OK\n");

    loop {
        led_set(true);
        delay(20_000);
        led_set(false);
        delay(20_000);
    }
}

unsafe fn gpio_uart_init() {
    let mut gpioa_crh = read_volatile(GPIOA_CRH);
    // PA9 = USART1_TX, alternate-function push-pull, 50 MHz (nibble 0xB).
    gpioa_crh = (gpioa_crh & !(0xF << 4)) | (0xB << 4);
    // PA10 = USART1_RX, floating input (nibble 0x4).
    gpioa_crh = (gpioa_crh & !(0xF << 8)) | (0x4 << 8);
    write_volatile(GPIOA_CRH, gpioa_crh);

    let mut gpioc_crh = read_volatile(GPIOC_CRH);
    // PC13 = low-speed open-drain output; the LED is wired to 3V3 and lights low.
    gpioc_crh = (gpioc_crh & !(0xF << 20)) | (0x6 << 20);
    write_volatile(GPIOC_CRH, gpioc_crh);

    // HSI reset clock is 8 MHz; BRR = 8 MHz / 115200 ≈ 0x45.
    write_volatile(USART1_BRR, 0x45);
    // UE | TE: enable USART1 and its transmitter.
    write_volatile(USART1_CR1, (1 << 13) | (1 << 3));
}

fn led_set(on: bool) {
    // PC13 sinks current: reset it low to light the LED, set it high to turn it off.
    let bits = if on {
        1 << (LED_PIN + 16)
    } else {
        1 << LED_PIN
    };
    unsafe { write_volatile(GPIOC_BSRR, bits) };
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            let mut remaining = 20_000;
            while read_volatile(USART1_SR) & TXE == 0 && remaining > 0 {
                remaining -= 1;
            }
            write_volatile(USART1_DR, *byte as u32);
        }
    }
}

fn delay(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}
