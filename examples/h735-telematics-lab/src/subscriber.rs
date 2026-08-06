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
const RCC_APB1LENR: u32 = RCC_BASE + 0xE8;
const RCC_APB2ENR: u32 = RCC_BASE + 0xF0;

const USART3_BASE: u32 = 0x4000_4800; // console
const USART1_BASE: u32 = 0x4001_1000; // modem

const ISR_TXE: u32 = 1 << 7;
const ISR_RXNE: u32 = 1 << 5;

#[inline(always)]
fn rd32(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}
#[inline(always)]
fn wr32(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
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
