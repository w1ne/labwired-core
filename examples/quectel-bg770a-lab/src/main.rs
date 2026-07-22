#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;

/// Enable USART1 (APB2 bit 14) and I2C1 (APB1 bit 21). Required now that
/// stm32f103.yaml clocks those peripherals — unclocked MMIO is dropped.
fn enable_peripheral_clocks() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 14)); // USART1EN
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 21)); // I2C1EN
    }
}


// USART1 on STM32F103: base 0x4001_3800 (uart1 in chip config) — talks to the modem.
const UART1_BASE: u32 = 0x4001_3800;
const UART1_SR: *const u32 = (UART1_BASE + 0x00) as *const u32;
const UART1_DR: *mut u32 = (UART1_BASE + 0x04) as *mut u32;

// USART2: base 0x4000_4400 — debug output, what the playground UART terminal shows.
const UART2_BASE: u32 = 0x4000_4400;
const UART2_SR: *const u32 = (UART2_BASE + 0x00) as *const u32;
const UART2_DR: *mut u32 = (UART2_BASE + 0x04) as *mut u32;

const SR_RXNE: u32 = 1 << 5;
const SR_TXE: u32 = 1 << 7;

fn uart_byte(sr: *const u32, dr: *mut u32, byte: u8) {
    unsafe {
        for _ in 0..256 {
            if core::ptr::read_volatile(sr) & SR_TXE != 0 {
                break;
            }
        }
        core::ptr::write_volatile(dr, byte as u32);
    }
}

fn uart_str(sr: *const u32, dr: *mut u32, s: &str) {
    for b in s.bytes() {
        uart_byte(sr, dr, b);
    }
}

fn uart1_str(s: &str) {
    uart_str(UART1_SR, UART1_DR, s);
}
fn uart2_str(s: &str) {
    uart_str(UART2_SR, UART2_DR, s);
}
fn uart2_byte(b: u8) {
    uart_byte(UART2_SR, UART2_DR, b);
}

fn uart1_has_data() -> bool {
    unsafe { core::ptr::read_volatile(UART1_SR) & SR_RXNE != 0 }
}

fn uart1_read_byte() -> u8 {
    unsafe { (core::ptr::read_volatile(UART1_DR) & 0xFF) as u8 }
}

/// Drain whatever the modem has sent so far. Caps loop iterations so a
/// chatty URC stream can't starve the rest of the program.
fn drain_modem_to_debug() {
    for _ in 0..4096 {
        if !uart1_has_data() {
            return;
        }
        let b = uart1_read_byte();
        uart2_byte(b);
    }
}

fn send_at(line: &str) {
    uart2_str("> ");
    uart2_str(line);
    uart2_str("\r\n");
    uart1_str(line);
    uart1_str("\r\n");
    for _ in 0..200_000 {
        drain_modem_to_debug();
    }
}

#[entry]
fn main() -> ! {
    
    enable_peripheral_clocks();
uart2_str("Quectel BG770A-GL modem lab\r\n");
    uart2_str("Driving the modem over UART1...\r\n\r\n");

    // Standard bring-up sequence.
    send_at("AT");
    send_at("ATE0");
    send_at("AT+CMEE=2");
    send_at("AT+CGMI");
    send_at("AT+CGMM");
    send_at("AT+CGSN");
    send_at("AT+CFUN?");
    send_at("AT+CPIN?");
    send_at("AT+CSQ");
    send_at("AT+QCSQ");
    send_at("AT+CEREG?");
    send_at("AT+CGATT?");

    uart2_str("\r\n[idle — modem URCs will stream through]\r\n");
    loop {
        drain_modem_to_debug();
    }
}
