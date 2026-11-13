// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM E70 Xplained smoke firmware.
//!
//! The board's ATSAME70Q21B is a Cortex-M7 with 2 MiB of flash at 0x00400000
//! and 384 KiB of SRAM at 0x20400000. The console is USART1 (the EDBG virtual
//! COM port: RXD1 on PA21, peripheral A; TXD1 on PB4, peripheral D) and the
//! user LED LED0 is PC8 (active low); the user button SW0 is PA11 (active low,
//! with the internal pull-up enabled here).
//!
//! Bring-up order, as on silicon:
//! 1. The watchdog runs from reset, so WDT_MR.WDDIS goes first (WDT_MR is
//!    write-once).
//! 2. PB4 resets as the JTAG TDI pin; MATRIX CCFG_SYSIO.SYSIO4 hands it to the
//!    PIO controller so the PIO can give it to USART1.
//! 3. The PMC gates PIOA/PIOB/PIOC/USART1 (peripheral ids 10/11/12/14).
//! 4. PA21 -> peripheral A and PB4 -> peripheral D via PIO_ABCDSR1/2 + PDR.
//! 5. USART1 is reset, set to 8N1 and started at 115384 baud on the 12 MHz
//!    main RC (CD = 6, FP = 4: 12e6 / (16 * 6.5)).
//!
//! The firmware never starts the crystal or the PLL, so the core runs at the
//! 12 MHz reset clock and the caches/TCM stay at their reset configuration.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// WDT @ 0x400E1850: MR @ 0x04 (WDDIS = bit 15).
const WDT_MR: *mut u32 = 0x400E_1854 as *mut u32;
const WDT_MR_WDDIS: u32 = 1 << 15;

// MATRIX @ 0x40088000: CCFG_SYSIO @ 0x114 (SYSIO4 = bit 4 selects PB4).
const MATRIX_CCFG_SYSIO: *mut u32 = 0x4008_8114 as *mut u32;
const CCFG_SYSIO_SYSIO4: u32 = 1 << 4;

// PMC @ 0x400E0600: PCER0 @ 0x10, PCSR0 @ 0x18, SR @ 0x68.
const PMC_PCER0: *mut u32 = 0x400E_0610 as *mut u32;
const PMC_PCSR0: *const u32 = 0x400E_0618 as *const u32;
const PMC_SR: *const u32 = 0x400E_0668 as *const u32;
const PMC_SR_MCKRDY: u32 = 1 << 3;

const ID_PIOA: u32 = 10;
const ID_PIOB: u32 = 11;
const ID_PIOC: u32 = 12;
const ID_USART1: u32 = 14;

// PIO register offsets (same block for PIOA..PIOE, 0x200 apart).
const PIOA: u32 = 0x400E_0E00;
const PIOB: u32 = 0x400E_1000;
const PIOC: u32 = 0x400E_1200;
const PIO_PER: u32 = 0x00;
const PIO_PDR: u32 = 0x04;
const PIO_PSR: u32 = 0x08;
const PIO_OER: u32 = 0x10;
const PIO_ODR: u32 = 0x14;
const PIO_SODR: u32 = 0x30;
const PIO_CODR: u32 = 0x34;
const PIO_ODSR: u32 = 0x38;
const PIO_PDSR: u32 = 0x3C;
const PIO_PUER: u32 = 0x60;
const PIO_ABCDSR1: u32 = 0x70;
const PIO_ABCDSR2: u32 = 0x74;

// USART1 @ 0x40028000: CR @ 0x00, MR @ 0x04, CSR @ 0x14, THR @ 0x1C,
// BRGR @ 0x20.
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
/// Normal mode, MCK clock, 8 data bits (CHRL = 3), no parity (PAR = 4),
/// 1 stop bit.
const US_MR_8N1: u32 = 0x8C0;
/// 12 MHz / (16 * (6 + 4/8)) = 115384 baud (+0.16 % against 115200).
const BRGR_CD: u32 = 6;
const BRGR_FP: u32 = 4;

const MAINCK_HZ: u32 = 12_000_000;

const LED_PIN: u32 = 1 << 8; // PC8, LED0, active low
const BUTTON_PIN: u32 = 1 << 11; // PA11, SW0, active low
const RXD1_PIN: u32 = 1 << 21; // PA21, peripheral A
const TXD1_PIN: u32 = 1 << 4; // PB4, peripheral D

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

fn put_hex(value: u32, digits: u32) {
    for shift in (0..digits).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

fn put_dec32(value: u32) {
    let mut digits = [0u8; 10];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
    }
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

fn odsr() -> u32 {
    rd(PIOC, PIO_ODSR)
}

fn led_pin_level() -> u32 {
    (rd(PIOC, PIO_PDSR) >> 8) & 1
}

fn button_level() -> u32 {
    (rd(PIOA, PIO_PDSR) >> 11) & 1
}

fn report(label: &str) {
    puts(label);
    puts(" pin=");
    put_dec32(led_pin_level());
    puts(" odsr=");
    put_hex(odsr(), 8);
    putc(b'\n');
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(WDT_MR, WDT_MR_WDDIS);
        let sysio = read_volatile(MATRIX_CCFG_SYSIO);
        write_volatile(MATRIX_CCFG_SYSIO, sysio | CCFG_SYSIO_SYSIO4);

        write_volatile(
            PMC_PCER0,
            (1 << ID_PIOA) | (1 << ID_PIOB) | (1 << ID_PIOC) | (1 << ID_USART1),
        );
        while read_volatile(PMC_SR) & PMC_SR_MCKRDY == 0 {}
    }

    // PA21 -> peripheral A (ABCDSR1/2 = 0/0), PB4 -> peripheral D (1/1).
    wr(PIOA, PIO_ABCDSR1, rd(PIOA, PIO_ABCDSR1) & !RXD1_PIN);
    wr(PIOA, PIO_ABCDSR2, rd(PIOA, PIO_ABCDSR2) & !RXD1_PIN);
    wr(PIOA, PIO_PDR, RXD1_PIN);
    wr(PIOB, PIO_ABCDSR1, rd(PIOB, PIO_ABCDSR1) | TXD1_PIN);
    wr(PIOB, PIO_ABCDSR2, rd(PIOB, PIO_ABCDSR2) | TXD1_PIN);
    wr(PIOB, PIO_PDR, TXD1_PIN);

    // USART1: reset and disable both directions, 8N1, baud, then enable.
    wr(
        USART1,
        US_CR,
        US_CR_RSTRX | US_CR_RSTTX | US_CR_RXDIS | US_CR_TXDIS,
    );
    wr(USART1, US_MR, US_MR_8N1);
    wr(USART1, US_BRGR, BRGR_CD | (BRGR_FP << 16));
    wr(USART1, US_CR, US_CR_RXEN | US_CR_TXEN);

    // PA11 input with the internal pull-up (SW0 is active low).
    wr(PIOA, PIO_PER, BUTTON_PIN);
    wr(PIOA, PIO_ODR, BUTTON_PIN);
    wr(PIOA, PIO_PUER, BUTTON_PIN);

    // PC8 as a driven output, latched high first so LED0 starts off.
    wr(PIOC, PIO_PER, LED_PIN);
    wr(PIOC, PIO_SODR, LED_PIN);
    wr(PIOC, PIO_OER, LED_PIN);

    puts("SAM E70 XPLAINED SMOKE\n");
    puts("cpu=cortex-m7 uart=usart1\n");
    puts("clk mainck=");
    put_dec32(MAINCK_HZ);
    puts(" cd=");
    put_dec32(BRGR_CD);
    puts(" fp=");
    put_dec32(BRGR_FP);
    puts(" baud=");
    put_dec32(MAINCK_HZ * 8 / (16 * (BRGR_CD * 8 + BRGR_FP)));
    putc(b'\n');

    puts("pmc pcsr0=");
    put_hex(unsafe { read_volatile(PMC_PCSR0) }, 8);
    putc(b'\n');

    puts("mux pa21 psr=");
    put_dec32((rd(PIOA, PIO_PSR) >> 21) & 1);
    puts(" pb4 psr=");
    put_dec32((rd(PIOB, PIO_PSR) >> 4) & 1);
    puts(" abcd=");
    put_dec32((rd(PIOB, PIO_ABCDSR2) >> 4) & 1);
    put_dec32((rd(PIOB, PIO_ABCDSR1) >> 4) & 1);
    putc(b'\n');

    report("led=off");
    wr(PIOC, PIO_CODR, LED_PIN);
    report("led=on");
    wr(PIOC, PIO_SODR, LED_PIN);
    report("led=off");

    puts("sw0=");
    put_dec32(button_level());
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        // Toggle LED0 once per heartbeat (on for even beats).
        if heartbeat & 1 == 0 {
            wr(PIOC, PIO_CODR, LED_PIN);
        } else {
            wr(PIOC, PIO_SODR, LED_PIN);
        }
        puts("hb=");
        put_dec32(heartbeat);
        puts(" odsr=");
        put_hex(odsr(), 8);
        puts(" sw0=");
        put_dec32(button_level());
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
