#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const UART0_DR: *mut u32 = 0x4007_0000 as *mut u32;
const ADC_CS: *mut u32 = 0x400A_0000 as *mut u32;
const ADC_RESULT: *const u32 = 0x400A_0004 as *const u32;
const GPIO25_CTRL: *mut u32 = 0x4002_80CC as *mut u32;
const SIO_GPIO_OUT_SET: *mut u32 = 0xD000_0014 as *mut u32;
const SIO_GPIO_OUT_CLR: *mut u32 = 0xD000_0018 as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = 0xD000_0024 as *mut u32;
const GPIO_FUNC_SIO: u32 = 5;
const LED_PIN: u32 = 25;

#[entry]
fn main() -> ! {
    let raw = unsafe {
        // Pico 2 GP26 is ADC0. The attached potentiometer defaults to its
        // midpoint: 1.65 V against the RP2350's 3.3 V ADC reference.
        core::ptr::write_volatile(ADC_CS, 1 | (1 << 2)); // EN | START_ONCE
        core::ptr::read_volatile(ADC_RESULT) & 0x0fff
    };

    print_uart("PICO2_ADC0=");
    print_decimal(raw);
    print_uart("\nPICO2_ADC_SMOKE_OK\n");

    unsafe {
        core::ptr::write_volatile(GPIO25_CTRL, GPIO_FUNC_SIO);
        core::ptr::write_volatile(SIO_GPIO_OE_SET, 1 << LED_PIN);
    }
    loop {
        unsafe {
            core::ptr::write_volatile(SIO_GPIO_OUT_SET, 1 << LED_PIN);
            delay(1_000);
            core::ptr::write_volatile(SIO_GPIO_OUT_CLR, 1 << LED_PIN);
            delay(1_000);
        }
    }
}

fn print_uart(s: &str) {
    for byte in s.bytes() {
        unsafe { core::ptr::write_volatile(UART0_DR, byte as u32) };
    }
}

fn print_decimal(mut value: u32) {
    let mut digits = [0u8; 10];
    let mut len = 0;
    loop {
        digits[len] = b'0' + (value % 10) as u8;
        len += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while len != 0 {
        len -= 1;
        unsafe { core::ptr::write_volatile(UART0_DR, digits[len] as u32) };
    }
}

fn delay(n: u32) {
    for _ in 0..n {
        cortex_m::asm::nop();
    }
}
