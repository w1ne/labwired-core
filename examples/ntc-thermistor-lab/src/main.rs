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

// RCC base: 0x4002_1000
const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;

// GPIOA CRL — the F1 pad mux for PA0..PA7. Four bits per pin, MODE[1:0] then
// CNF[1:0]. This family has no `MODER` and no `AFR`, so there is no AF number
// to write: a pin's alternate function is fixed by the pin.
const GPIOA_CRL: *mut u32 = (0x4001_0800 + 0x00) as *mut u32;

// USART2 base: 0x4000_4400 (debug output)
const UART2_BASE: u32 = 0x4000_4400;
const UART2_SR: *const u32 = (UART2_BASE + 0x00) as *const u32;
const UART2_DR: *mut u32 = (UART2_BASE + 0x04) as *mut u32;
const UART2_BRR: *mut u32 = (UART2_BASE + 0x08) as *mut u32;
const UART2_CR1: *mut u32 = (UART2_BASE + 0x0C) as *mut u32;

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

/// Clock USART2, mux PA2 to `USART2_TX`, and give the transmitter a divisor.
///
/// This lab used to write `UART2_DR` with no setup at all. That transmits on
/// LabWired's permissive USART model and nowhere else: PA2 stays the floating
/// input it is after reset, so the pad route never goes live and a logic
/// analyzer on PA2 reads the GPIO latch — a flat line — while the
/// transaction-level bus monitor decodes the same traffic fine. A zero `BRR` is
/// the other half: with no divisor there is no bit period, so there is nothing
/// to narrate even once a route exists.
///
/// * PA2 = `USART2_TX` in the **Default** alternate-function column
///   (DS5319 Rev 20, Table 5, p.29), so no AFIO remap is involved.
/// * The `CRL` nibble for PA2 is bits [11:8]. `0xB` = MODE `0b11` (output,
///   50 MHz) + CNF `0b10` (alternate-function push-pull).
/// * `BRR` = f_PCLK1 / baud at the default 16× oversampling. This firmware
///   never touches the PLL, so the part runs on the 8 MHz HSI it selects at
///   reset (DS5319 Rev 20 §2.3.7, p.15) with an APB1 prescaler of 1:
///   8_000_000 / 115_200 = 69.44 → 69 = 0x45.
/// * `CR1` = UE (bit 13) | TE (bit 3): transmit only, no interrupts.
///
/// AFIOEN (APB2 bit 0) and IOPAEN (bit 2) come first — without IOPAEN the GPIO
/// port is held in reset and the `CRL` write below would be swallowed.
fn uart2_init() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 0) | (1 << 2)); // AFIOEN | IOPAEN
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 17)); // USART2EN

        let crl = core::ptr::read_volatile(GPIOA_CRL);
        core::ptr::write_volatile(GPIOA_CRL, (crl & !(0xF << 8)) | (0xB << 8));

        core::ptr::write_volatile(UART2_BRR, 0x45);
        core::ptr::write_volatile(UART2_CR1, (1 << 13) | (1 << 3));
    }
}

/// Configure ADC1 channel 0.
fn adc1_init() {
    unsafe {
        // Enable the ADC1 clock via RCC_APB2ENR (bit 9 = ADC1EN, RM0008 §7.3.7).
        // ADC1 is unclocked out of reset, and LabWired's chip YAML gates the
        // peripheral the same way silicon does: with the clock off every ADC1
        // register read returns 0 and every write is dropped, so SR never
        // raises EOC and conversions never complete.
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 9));
        // Set ADC CR1: no interrupts, single channel mode.
        core::ptr::write_volatile(ADC1_CR1, 0);
        // CR2: ADON = 0 initially; software trigger (EXTSEL = 0b111, EXTTRIG = 1).
        core::ptr::write_volatile(ADC1_CR2, 0);
    }
}

#[entry]
fn main() -> ! {
    uart2_init();

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
