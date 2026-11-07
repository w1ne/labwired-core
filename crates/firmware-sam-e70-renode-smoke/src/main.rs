// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Focused smoke for the imported SAM E70 profile. It touches only the modeled
// PMC, PIOC, and USART1 paths; watchdog and matrix accesses are outside this
// profile and are intentionally absent.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const PMC_PCER0: *mut u32 = 0x400E_0610 as *mut u32;
const PMC_PCSR0: *const u32 = 0x400E_0618 as *const u32;
const PMC_SR: *const u32 = 0x400E_0668 as *const u32;
const PMC_SR_MCKRDY: u32 = 1 << 3;
const ID_PIOC: u32 = 12;
const ID_USART1: u32 = 14;

const PIOC: u32 = 0x400E_1200;
const PIO_PER: u32 = 0x00;
const PIO_OER: u32 = 0x10;
const PIO_SODR: u32 = 0x30;
const PIO_CODR: u32 = 0x34;
const PIO_ODSR: u32 = 0x38;
const LED_PIN: u32 = 1 << 8;

const USART1: u32 = 0x4002_8000;
const US_CR: u32 = 0x00;
const US_MR: u32 = 0x04;
const US_CSR: u32 = 0x14;
const US_THR: u32 = 0x1C;
const US_BRGR: u32 = 0x20;
const US_CR_RSTRX: u32 = 1 << 2;
const US_CR_RSTTX: u32 = 1 << 3;
const US_CR_RXEN: u32 = 1 << 4;
const US_CR_RXDIS: u32 = 1 << 5;
const US_CR_TXEN: u32 = 1 << 6;
const US_CR_TXDIS: u32 = 1 << 7;
const US_CSR_TXRDY: u32 = 1 << 1;
const US_MR_8N1: u32 = 0x8C0;

fn reg(base: u32, offset: u32) -> *mut u32 {
    (base + offset) as *mut u32
}

fn rd(base: u32, offset: u32) -> u32 {
    unsafe { read_volatile(reg(base, offset)) }
}

fn wr(base: u32, offset: u32, value: u32) {
    unsafe { write_volatile(reg(base, offset), value) }
}

fn putc(byte: u8) {
    while rd(USART1, US_CSR) & US_CSR_TXRDY == 0 {}
    wr(USART1, US_THR, byte as u32);
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

fn put_hex8(value: u32) {
    for shift in (0..8).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        });
    }
}

fn report_led(label: &str) {
    puts(label);
    puts(" odsr=");
    put_hex8(rd(PIOC, PIO_ODSR));
    putc(b'\n');
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(PMC_PCER0, (1 << ID_PIOC) | (1 << ID_USART1));
        while read_volatile(PMC_SR) & PMC_SR_MCKRDY == 0 {}
    }

    // The profile tests the PIO latch and USART model directly. It does not
    // claim physical pad muxing or the matrix's PB4/TDI selection behavior.
    wr(PIOC, PIO_PER, LED_PIN);
    wr(PIOC, PIO_SODR, LED_PIN);
    wr(PIOC, PIO_OER, LED_PIN);

    wr(
        USART1,
        US_CR,
        US_CR_RSTRX | US_CR_RSTTX | US_CR_RXDIS | US_CR_TXDIS,
    );
    wr(USART1, US_MR, US_MR_8N1);
    wr(USART1, US_BRGR, 6 | (4 << 16));
    wr(USART1, US_CR, US_CR_RXEN | US_CR_TXEN);

    puts("SAM E70 RENODE PROFILE SMOKE\n");
    puts("cpu=cortex-m7 uart=usart1\n");
    puts("pmc pcsr0=");
    put_hex8(unsafe { read_volatile(PMC_PCSR0) });
    putc(b'\n');
    report_led("led=off");

    wr(PIOC, PIO_CODR, LED_PIN);
    report_led("led=on");
    wr(PIOC, PIO_SODR, LED_PIN);
    report_led("led=off");

    loop {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}
