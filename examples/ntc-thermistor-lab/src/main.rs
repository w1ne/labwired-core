#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use cortex_m_rt::entry;
use panic_halt as _;

// STM32F103 register addresses
// ADC1 base: 0x4001_2400
const ADC1_BASE: u32 = 0x4001_2400;
const ADC1_SR: *mut u32 = (ADC1_BASE + 0x00) as *mut u32;
const ADC1_CR1: *mut u32 = (ADC1_BASE + 0x04) as *mut u32;
const ADC1_CR2: *mut u32 = (ADC1_BASE + 0x08) as *mut u32;
const ADC1_DR: *const u32 = (ADC1_BASE + 0x4C) as *const u32;

// USART2 base: 0x4000_4400 (debug output)
const UART2_BASE: u32 = 0x4000_4400;
const UART2_SR: *const u32 = (UART2_BASE + 0x00) as *const u32;
const UART2_DR: *mut u32 = (UART2_BASE + 0x04) as *mut u32;

const SR_EOC: u32 = 1 << 1; // End of conversion
const SR_TXE: u32 = 1 << 7; // UART TX empty

fn uart2_byte(byte: u8) {
    unsafe {
        for _ in 0..256 {
            if core::ptr::read_volatile(UART2_SR) & SR_TXE != 0 {
                break;
            }
        }
        core::ptr::write_volatile(UART2_DR, byte as u32);
    }
}

fn uart2_str(s: &str) {
    for b in s.bytes() {
        uart2_byte(b);
    }
}

/// Print a u32 decimal value to UART2.
fn uart2_u32(mut n: u32) {
    if n == 0 {
        uart2_byte(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in (0..i).rev() {
        uart2_byte(buf[j]);
    }
}

/// Trigger a single ADC1 conversion and return the 12-bit result.
fn adc1_read() -> u16 {
    unsafe {
        // Enable ADC (ADON = bit 0)
        core::ptr::write_volatile(ADC1_CR2, 1);
        // Trigger SW start (SWSTART = bit 30)
        core::ptr::write_volatile(ADC1_CR2, 1 | (1 << 30));
        // Wait for EOC
        let mut timeout = 100_000u32;
        loop {
            if core::ptr::read_volatile(ADC1_SR) & SR_EOC != 0 {
                break;
            }
            timeout -= 1;
            if timeout == 0 {
                return 0;
            }
        }
        // Read DR clears EOC on STM32F1
        (core::ptr::read_volatile(ADC1_DR) & 0xFFF) as u16
    }
}

/// Configure ADC1 channel 0.
fn adc1_init() {
    unsafe {
        // Enable ADC clock via RCC_APB2ENR (bit 9 = ADC1EN).
        // Skipped in sim — peripheral is always available.
        // Set ADC CR1: no interrupts, single channel mode.
        core::ptr::write_volatile(ADC1_CR1, 0);
        // CR2: ADON = 0 initially; software trigger (EXTSEL = 0b111, EXTTRIG = 1).
        core::ptr::write_volatile(ADC1_CR2, 0);
    }
}

#[entry]
fn main() -> ! {
    uart2_str("NTC Thermistor Lab\r\n");
    uart2_str("ADC1 ch0 -> 12-bit count (0..4095)\r\n");
    uart2_str("Slide the temperature slider in the inspector to see the count change.\r\n");

    adc1_init();

    let mut iteration = 0u32;

    loop {
        let count = adc1_read();

        uart2_str("[NTC] iter=");
        uart2_u32(iteration);
        uart2_str(" adc=");
        uart2_u32(count as u32);
        uart2_str("/4095\r\n");

        iteration += 1;

        // Busy-wait between readings so the serial monitor is readable.
        for _ in 0..200_000u32 {
            unsafe { core::ptr::read_volatile(ADC1_SR) };
        }
    }
}
