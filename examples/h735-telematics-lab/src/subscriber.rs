//! H735 telematics **subscriber** UE — pairs with `main.rs` publisher on shared fabric.
//!
//! Opens MQTT AT, subscribes to `telematics/#`, waits for fabric fan-out
//! (`+QMTRECV`), prints `location received`. No TFT / GPS path — keeps the
//! dual-UE story honest: one node **sends**, one node **collects**.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x5802_4400;
const RCC_AHB4ENR: u32 = RCC_BASE + 0xE0;
const RCC_APB1LENR: u32 = RCC_BASE + 0xE8;
const RCC_APB2ENR: u32 = RCC_BASE + 0xF0;

const GPIOA_BASE: u32 = 0x5802_0000;
const GPIOB_BASE: u32 = 0x5802_0400;

const GPIO_MODER: u32 = 0x00;
const GPIO_OTYPER: u32 = 0x04;
const GPIO_OSPEEDR: u32 = 0x08;
const GPIO_PUPDR: u32 = 0x0C;
const GPIO_AFR: u32 = 0x20;

const USART3_BASE: u32 = 0x4000_4800; // console
const USART1_BASE: u32 = 0x4001_1000; // modem

const USART_CR1: u32 = 0x00;
const USART_BRR: u32 = 0x0C;
const CR1_UE: u32 = 1 << 0;
const CR1_RE: u32 = 1 << 2;
const CR1_TE: u32 = 1 << 3;
const ISR_TXE: u32 = 1 << 7;
const ISR_RXNE: u32 = 1 << 5;

/// 115200 8N1 off the 64 MHz HSI reset clock, exactly as in `main.rs` — see the
/// arithmetic there. USARTDIV = 64_000_000 / 115_200 = 555.6 → 556.
const BRR_115200: u32 = 556;

/// PA9/PA10 = USART1_TX/RX and PB10/PB11 = USART3_TX/RX, all AF7 — STM32H735xG
/// datasheet DS13312 Rev 4, Table 9, port A p97 and port B p99.
const AF7: u32 = 7;

#[inline(always)]
fn rd32(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}
#[inline(always)]
fn wr32(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// Hand one pad to a USART: MODER = 10 (alternate function), push-pull, very
/// high speed, no pull, and the AF nibble into AFR[pin / 8]. Without the AF
/// nibble the pin stays a plain GPIO and a logic probe reads the output latch
/// instead of the serial waveform.
fn pad_af(gpio_base: u32, pin: u32, af: u32) {
    let shift = pin * 2;
    let moder = rd32(gpio_base + GPIO_MODER);
    wr32(
        gpio_base + GPIO_MODER,
        (moder & !(0b11 << shift)) | (0b10 << shift),
    );
    wr32(
        gpio_base + GPIO_OTYPER,
        rd32(gpio_base + GPIO_OTYPER) & !(1 << pin),
    );
    wr32(
        gpio_base + GPIO_OSPEEDR,
        rd32(gpio_base + GPIO_OSPEEDR) | (0b11 << shift),
    );
    wr32(
        gpio_base + GPIO_PUPDR,
        rd32(gpio_base + GPIO_PUPDR) & !(0b11 << shift),
    );
    let afr = gpio_base + GPIO_AFR + (pin >> 3) * 4;
    let nib = (pin & 7) * 4;
    wr32(afr, (rd32(afr) & !(0xF << nib)) | (af << nib));
}

/// Mux both UARTs onto their pads and give each one a real bit period. The H7
/// USART kernel clock needs no selection: RCC_D2CCIP2R resets to rcc_pclk1/2
/// (RM0468 §9.7.21), the buses this firmware leaves at their reset rate.
fn uart_pads_init() {
    wr32(RCC_AHB4ENR, rd32(RCC_AHB4ENR) | (1 << 0) | (1 << 1));

    pad_af(GPIOA_BASE, 9, AF7); // USART1_TX → modem
    pad_af(GPIOA_BASE, 10, AF7); // USART1_RX → modem
    pad_af(GPIOB_BASE, 10, AF7); // USART3_TX → console
    pad_af(GPIOB_BASE, 11, AF7); // USART3_RX → console

    for base in [USART1_BASE, USART3_BASE] {
        wr32(base + USART_CR1, 0);
        wr32(base + USART_BRR, BRR_115200);
        wr32(base + USART_CR1, CR1_UE | CR1_TE | CR1_RE);
    }
}

fn console_byte(b: u8) {
    for _ in 0..10_000 {
        if rd32(USART3_BASE + 0x1C) & ISR_TXE != 0 {
            break;
        }
    }
    unsafe { write_volatile((USART3_BASE + 0x28) as *mut u8, b) };
}

fn console_str(s: &str) {
    for b in s.bytes() {
        console_byte(b);
    }
}

fn modem_byte(b: u8) {
    for _ in 0..10_000 {
        if rd32(USART1_BASE + 0x1C) & ISR_TXE != 0 {
            break;
        }
    }
    unsafe { write_volatile((USART1_BASE + 0x28) as *mut u8, b) };
}

fn modem_str(s: &str) {
    for b in s.bytes() {
        modem_byte(b);
    }
}

fn modem_has_rx() -> bool {
    rd32(USART1_BASE + 0x1C) & ISR_RXNE != 0
}

fn modem_read() -> u8 {
    (rd32(USART1_BASE + 0x24) & 0xFF) as u8
}

fn drain_modem(buf: &mut [u8], len: &mut usize) {
    for _ in 0..8192 {
        if !modem_has_rx() {
            return;
        }
        let b = modem_read();
        console_byte(b);
        if *len < buf.len() {
            buf[*len] = b;
            *len += 1;
        } else {
            buf.copy_within(1.., 0);
            buf[buf.len() - 1] = b;
        }
    }
}

fn buf_contains(buf: &[u8], n: usize, needle: &[u8]) -> bool {
    if needle.is_empty() || n < needle.len() {
        return false;
    }
    'outer: for i in 0..=(n - needle.len()) {
        for j in 0..needle.len() {
            if buf[i + j] != needle[j] {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

fn send_at_until(line: &str, buf: &mut [u8], len: &mut usize, needle: &[u8], iters: u32) {
    *len = 0;
    console_str("> ");
    console_str(line);
    console_str("\r\n");
    modem_str(line);
    modem_str("\r\n");
    for _ in 0..iters {
        drain_modem(buf, len);
        if !needle.is_empty() && buf_contains(buf, *len, needle) {
            break;
        }
    }
}

fn send_at(line: &str, buf: &mut [u8], len: &mut usize) {
    send_at_until(line, buf, len, b"OK", 200_000);
}

#[entry]
fn main() -> ! {
    wr32(RCC_APB1LENR, rd32(RCC_APB1LENR) | (1 << 18));
    wr32(RCC_APB2ENR, rd32(RCC_APB2ENR) | (1 << 4));
    uart_pads_init();

    console_str("LabWired telematics subscriber / H735 + modem\r\n");
    console_str("Collect: QMTSUB telematics/# → wait +QMTRECV from fabric\r\n\r\n");

    let mut resp = [0u8; 512];
    let mut resp_len = 0usize;

    send_at("AT", &mut resp, &mut resp_len);
    send_at("ATE0", &mut resp, &mut resp_len);
    send_at("AT+CMEE=2", &mut resp, &mut resp_len);
    send_at("AT+CSQ", &mut resp, &mut resp_len);
    send_at("AT+CGATT?", &mut resp, &mut resp_len);

    send_at_until(
        "AT+QMTOPEN=0,\"broker.labwired.local\",1883",
        &mut resp,
        &mut resp_len,
        b"+QMTOPEN: 0,0",
        2_000_000,
    );
    send_at_until(
        "AT+QMTCONN=0,\"labwired-subscriber\"",
        &mut resp,
        &mut resp_len,
        b"+QMTCONN:",
        500_000,
    );
    send_at_until(
        "AT+QMTSUB=0,1,\"telematics/#\",0",
        &mut resp,
        &mut resp_len,
        b"+QMTSUB:",
        500_000,
    );

    console_str("subscribed — waiting for fabric delivery\r\n");

    // World interleaves both UEs; peer publishes after GPS/TFT. Drain until
    // +QMTRECV or a generous spin budget elapses.
    resp_len = 0;
    let mut got = false;
    for _ in 0..8_000_000 {
        drain_modem(&mut resp, &mut resp_len);
        if buf_contains(&resp, resp_len, b"+QMTRECV:") {
            got = true;
            break;
        }
    }

    if got {
        console_str("location received\r\n");
    } else {
        console_str("location timeout (no +QMTRECV)\r\n");
    }

    loop {
        for _ in 0..500_000 {
            drain_modem(&mut resp, &mut resp_len);
        }
    }
}
