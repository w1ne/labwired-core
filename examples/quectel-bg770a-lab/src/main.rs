#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;

/// GPIOA CRL/CRH — the F1 pad mux for PA0..PA7 and PA8..PA15. Four bits per
/// pin, MODE[1:0] then CNF[1:0]. This family has no `MODER` and no `AFR`, so
/// there is no AF number to write: a pin's alternate function is fixed.
const GPIOA_CRL: *mut u32 = (0x4001_0800 + 0x00) as *mut u32;
const GPIOA_CRH: *mut u32 = (0x4001_0800 + 0x04) as *mut u32;

/// Enable AFIO (APB2 bit 0), GPIOA (bit 2), USART1 (bit 14), USART2 (APB1
/// bit 17) and I2C1 (APB1 bit 21). Required now that stm32f103.yaml clocks
/// those peripherals — unclocked MMIO is dropped.
///
/// AFIOEN, IOPAEN and USART2EN were all missing. Without IOPAEN the GPIO port
/// is held in reset, so `serial_init`'s CRL/CRH writes would be swallowed and
/// PA2/PA9 would stay the floating inputs they are after reset.
fn enable_peripheral_clocks() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        // AFIOEN | IOPAEN | USART1EN
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 0) | (1 << 2) | (1 << 14));
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        // USART2EN | I2C1EN
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 17) | (1 << 21));
    }
}

/// Mux the serial pads and give both transmitters a baud divisor.
///
/// Clocking the USARTs and writing `DR` is all this lab used to do, and it is
/// enough for LabWired's permissive USART model. It is not enough for silicon
/// and it is not enough for a probe: PA2 and PA9 stay floating inputs until
/// CRL/CRH select an alternate-function output, so the pad routes never go live
/// and a logic analyzer on either pin reads the GPIO latch — a flat line —
/// while the transaction-level bus monitor decodes the same traffic fine. A
/// zero `BRR` is the other half: with no divisor there is no bit period, so
/// there is nothing to narrate even once a route exists.
///
/// Pads, from the **Default** alternate-function column of DS5319 Rev 20,
/// Table 5 — no AFIO remap is involved for any of them:
///
/// * PA9  = `USART1_TX` (p.31) → CRH bits [7:4]   = `0xB`
/// * PA10 = `USART1_RX` (p.31) → CRH bits [11:8]  = `0x4`
/// * PA2  = `USART2_TX` (p.29) → CRL bits [11:8]  = `0xB`
///
/// `0xB` is MODE `0b11` (output, 50 MHz) + CNF `0b10` (alternate-function
/// push-pull); `0x4` is MODE `0b00` (input) + CNF `0b01` (floating), which is
/// what a receive pin needs.
///
/// `BRR` = f_PCLK / baud at the default 16× oversampling. This firmware never
/// touches the PLL, so the part runs on the 8 MHz HSI it selects at reset
/// (DS5319 Rev 20 §2.3.7, p.15) and both APB prescalers are 1, so PCLK1 =
/// PCLK2 = 8 MHz.
///
/// * USART1 (the modem AT link) at 115 200: 8_000_000 / 115_200 = 69.44 →
///   69 = 0x45. The BG77xA-GL LPWA specification V1.7 in the datasheet corpus
///   lists the UART count but not its default rate, so 115 200 is this
///   firmware's choice for the AT link rather than a cited module default.
/// * USART2 (console) at 115 200: 8_000_000 / 115_200 = 69.44 → 69 = 0x45.
fn serial_init() {
    unsafe {
        let crl = core::ptr::read_volatile(GPIOA_CRL);
        core::ptr::write_volatile(GPIOA_CRL, (crl & !(0xF << 8)) | (0xB << 8));
        let crh = core::ptr::read_volatile(GPIOA_CRH);
        let crh = (crh & !(0xFF << 4)) | (0xB << 4) | (0x4 << 8);
        core::ptr::write_volatile(GPIOA_CRH, crh);

        core::ptr::write_volatile(UART1_BRR, 0x45);
        // UE (bit 13) | TE (bit 3) | RE (bit 2) — this link is bidirectional.
        core::ptr::write_volatile(UART1_CR1, (1 << 13) | (1 << 3) | (1 << 2));

        core::ptr::write_volatile(UART2_BRR, 0x45);
        // UE | TE — the console is transmit only.
        core::ptr::write_volatile(UART2_CR1, (1 << 13) | (1 << 3));
    }
}

// USART1 on STM32F103: base 0x4001_3800 (uart1 in chip config) — talks to the modem.
const UART1_BASE: u32 = 0x4001_3800;
const UART1_SR: *const u32 = (UART1_BASE + 0x00) as *const u32;
const UART1_DR: *mut u32 = (UART1_BASE + 0x04) as *mut u32;
const UART1_BRR: *mut u32 = (UART1_BASE + 0x08) as *mut u32;
const UART1_CR1: *mut u32 = (UART1_BASE + 0x0C) as *mut u32;

// USART2: base 0x4000_4400 — debug output, what the playground UART terminal shows.
const UART2_BASE: u32 = 0x4000_4400;
const UART2_SR: *const u32 = (UART2_BASE + 0x00) as *const u32;
const UART2_DR: *mut u32 = (UART2_BASE + 0x04) as *mut u32;
const UART2_BRR: *mut u32 = (UART2_BASE + 0x08) as *mut u32;
const UART2_CR1: *mut u32 = (UART2_BASE + 0x0C) as *mut u32;

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
    serial_init();
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
