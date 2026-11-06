// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! FocalTech FT9001 (Cortex-M4F) onboarding smoke: USART2 polled TX/RX in
//! bare registers.
//!
//! The register block is the one Renode's `UART.FT9001_USART` model exposes
//! (renode-infrastructure `FT9001_USART.cs`), at USART2 0x4001_4000:
//!   BDL 0x00, BDH 0x01, CR2 0x02 (RE bit 2, TE bit 3), CR1 0x03,
//!   DRL 0x06, BRDF 0x0A, FCR 0x0E, FSR 0x11 (REMPTY 0, TEMPTY 2, TFULL 3,
//!   RFTS 5, FTC 6), FCR2 0x13 (RXFCLR bit 0, write-only).
//! The BDL/BDH/BRDF values written here are read-back probes only; no baud
//! rate is derived from them.
//!
//! The smoke script injects "AB" at start and "Z" later. The run proves:
//! a byte written before TE is dropped, control registers read back, FSR
//! tracks the RX FIFO, RXFCLR empties it, a later byte is received, and
//! clearing TE drops TX again.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const USART2: u32 = 0x4001_4000;
const BDL: u32 = USART2;
const BDH: u32 = USART2 + 0x01;
const CR2: u32 = USART2 + 0x02;
const CR1: u32 = USART2 + 0x03;
const DRL: u32 = USART2 + 0x06;
const BRDF: u32 = USART2 + 0x0A;
const FCR: u32 = USART2 + 0x0E;
const FSR: u32 = USART2 + 0x11;
const FCR2: u32 = USART2 + 0x13;

const CR2_RE: u8 = 1 << 2;
const CR2_TE: u8 = 1 << 3;
const FCR_TFEN: u8 = 1 << 6;
const FCR_RFEN: u8 = 1 << 7;
const FCR2_RXFCLR: u8 = 1 << 0;
const FSR_TFULL: u8 = 1 << 3;
const FSR_RFTS: u8 = 1 << 5;

fn rd(addr: u32) -> u8 {
    unsafe { read_volatile(addr as *const u8) }
}

fn wr(addr: u32, value: u8) {
    unsafe { write_volatile(addr as *mut u8, value) }
}

fn putc(c: u8) {
    while rd(FSR) & FSR_TFULL != 0 {}
    wr(DRL, c);
}

fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

fn hex8(v: u8) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    putc(DIGITS[(v >> 4) as usize]);
    putc(DIGITS[(v & 0xF) as usize]);
}

fn field(name: &str, v: u8) {
    puts(name);
    putc(b'=');
    hex8(v);
}

#[entry]
fn main() -> ! {
    // TE is clear out of reset: this byte must not reach the line.
    wr(DRL, b'X');

    wr(CR2, CR2_TE | CR2_RE);
    puts("FT9001 SMOKE\r\n");
    puts("cpu=cortex-m4f uart=usart2\r\n");

    wr(BDL, 0x56);
    wr(BDH, 0x01);
    wr(BRDF, 0x1A);
    wr(CR1, 0x00);
    wr(FCR, FCR_TFEN | FCR_RFEN);
    field("cr2", rd(CR2));
    field(" cr1", rd(CR1));
    field(" bdh", rd(BDH));
    field(" bdl", rd(BDL));
    field(" brdf", rd(BRDF));
    field(" fcr", rd(FCR));
    puts("\r\n");

    // "AB" was injected before the first instruction.
    field("rx fsr", rd(FSR));
    let first = rd(DRL);
    puts(" drl=");
    putc(first);
    field(" fsr", rd(FSR));
    puts("\r\n");

    wr(FCR2, FCR2_RXFCLR);
    field("rxfclr fsr", rd(FSR));
    field(" drl", rd(DRL));
    field(" fcr2", rd(FCR2));
    puts("\r\n");

    // The second injection lands later in the run.
    while rd(FSR) & FSR_RFTS == 0 {}
    let late = rd(DRL);
    puts("late drl=");
    putc(late);
    field(" fsr", rd(FSR));
    puts("\r\n");

    puts("te=off");
    wr(CR2, CR2_RE);
    wr(DRL, b'Y');
    wr(CR2, CR2_TE | CR2_RE);
    puts("|te=on\r\n");

    puts("FT9001 SMOKE DONE\r\n");
    loop {
        core::hint::spin_loop();
    }
}
