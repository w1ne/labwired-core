// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! FRDM-K64F (MK64FN1M0VLL12, Cortex-M4F) UART-only L1 smoke: a running light
//! over the RGB LED plus a heartbeat on UART0, in bare registers.
//!
//! Board facts (NXP FRDM-K64F user manual; Zephyr `frdm_k64f`):
//!   red   PTB22 (LD1, active low)
//!   green PTE26 (LD2, active low)
//!   blue  PTB21 (LD3, active low)
//!   SW2 PTC6, SW3 PTA4 (active low, not read here)
//!   UART0 PTB16 (RX) / PTB17 (TX), ALT3, OpenSDA VCP, 115200 8N1
//!
//! The firmware runs on the MCG FEI reset clock (32.768 kHz IRC x 640 =
//! 20.97152 MHz). No PLL/MCG change is made: UART0's reset clock source is
//! MCGFLLCLK. BR = 20.97152 MHz / (16 * 115200) = 11.375 → BDL 11, BRFA 12.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// SIM (K64 RM §12): SCGC4 @ 0x1034, SCGC5 @ 0x1038.
const SIM: u32 = 0x4004_7000;
const SIM_SCGC4: *mut u32 = (SIM + 0x1034) as *mut u32;
const SIM_SCGC5: *mut u32 = (SIM + 0x1038) as *mut u32;

// PORTB / PORTE (pin mux): PCRn at 0x00 + 4*n. MUX field is bits 8..10.
const PORTB: u32 = 0x4004_A000;
const PORTE: u32 = 0x4004_D000;
const PCR_MUX_GPIO: u32 = 0b001 << 8;
const PCR_MUX_UART0: u32 = 0b011 << 8;

// GPIOB / GPIOE (Kinetis GPIO): PDOR @ 0x00, PSOR @ 0x04, PCOR @ 0x08,
// PDDR @ 0x14.
const GPIOB: u32 = 0x400F_F040;
const GPIOE: u32 = 0x400F_F100;
const PDOR: u32 = 0x00;
const PSOR: u32 = 0x04;
const PCOR: u32 = 0x08;
const PDDR: u32 = 0x14;

const RED_PIN: u32 = 22; // PTB22
const GREEN_PIN: u32 = 26; // PTE26
const BLUE_PIN: u32 = 21; // PTB21

// UART0: BDH @ 0x00, BDL @ 0x01, C1 @ 0x02, C2 @ 0x03, S1 @ 0x04, D @ 0x07,
// C4 @ 0x0A.
const UART0: u32 = 0x4006_A000;
const UART0_BDH: *mut u8 = UART0 as *mut u8;
const UART0_BDL: *mut u8 = (UART0 + 0x01) as *mut u8;
const UART0_C1: *mut u8 = (UART0 + 0x02) as *mut u8;
const UART0_C2: *mut u8 = (UART0 + 0x03) as *mut u8;
const UART0_S1: *const u8 = (UART0 + 0x04) as *const u8;
const UART0_D: *mut u8 = (UART0 + 0x07) as *mut u8;
const UART0_C4: *mut u8 = (UART0 + 0x0A) as *mut u8;

const UART_S1_TDRE: u8 = 1 << 7;
const UART_C2_RE: u8 = 1 << 2;
const UART_C2_TE: u8 = 1 << 3;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UART0_S1) & UART_S1_TDRE == 0 {}
        write_volatile(UART0_D, byte);
    }
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
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

fn port_pcr(base: u32, pin: u32, mux: u32) {
    unsafe { write_volatile((base + 4 * pin) as *mut u32, mux) };
}

fn gpio_set_dir(base: u32, mask: u32) {
    unsafe {
        let p = (base + PDDR) as *mut u32;
        write_volatile(p, read_volatile(p) | mask);
    }
}

fn gpio_set(base: u32, mask: u32) {
    unsafe { write_volatile((base + PSOR) as *mut u32, mask) };
}

fn gpio_clear(base: u32, mask: u32) {
    unsafe { write_volatile((base + PCOR) as *mut u32, mask) };
}

fn pin_level(base: u32, pin: u32) -> u32 {
    unsafe { (read_volatile((base + PDOR) as *const u32) >> pin) & 1 }
}

fn report(name: &[u8], base: u32, pin: u32, on: bool) {
    puts(b"led=");
    puts(name);
    puts(if on { b" on pin=" } else { b" off pin=" });
    put_dec32(pin_level(base, pin));
    putc(b'\n');
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // UART0 on SCGC4 bit 10; PORTB/PORTE on SCGC5 bits 10/13.
        write_volatile(SIM_SCGC4, read_volatile(SIM_SCGC4) | (1 << 10));
        write_volatile(SIM_SCGC5, read_volatile(SIM_SCGC5) | (1 << 10) | (1 << 13));

        // PTB16/PTB17 = UART0 ALT3; PTB22/PTB21/PTE26 = GPIO ALT1.
        port_pcr(PORTB, 16, PCR_MUX_UART0);
        port_pcr(PORTB, 17, PCR_MUX_UART0);
        port_pcr(PORTB, 22, PCR_MUX_GPIO);
        port_pcr(PORTB, 21, PCR_MUX_GPIO);
        port_pcr(PORTE, 26, PCR_MUX_GPIO);

        // LEDs as outputs, initially off (active low → drive high).
        let pb_mask = (1 << RED_PIN) | (1 << BLUE_PIN);
        let pe_mask = 1 << GREEN_PIN;
        gpio_set_dir(GPIOB, pb_mask);
        gpio_set_dir(GPIOE, pe_mask);
        gpio_set(GPIOB, pb_mask);
        gpio_set(GPIOE, pe_mask);

        // UART0: 8N1, 115200 from 20.97152 MHz (BDL 11, BRFA 12/32).
        write_volatile(UART0_BDH, 0);
        write_volatile(UART0_BDL, 11);
        write_volatile(UART0_C4, 12);
        write_volatile(UART0_C1, 0);
        write_volatile(UART0_C2, UART_C2_RE | UART_C2_TE);
    }

    puts(b"FRDM-K64F SMOKE\n");
    puts(b"cpu=cortex-m4 uart=uart0\n");

    // Red.
    gpio_clear(GPIOB, 1 << RED_PIN);
    report(b"red", GPIOB, RED_PIN, true);
    gpio_set(GPIOB, 1 << RED_PIN);
    report(b"red", GPIOB, RED_PIN, false);

    // Green.
    gpio_clear(GPIOE, 1 << GREEN_PIN);
    report(b"green", GPIOE, GREEN_PIN, true);
    gpio_set(GPIOE, 1 << GREEN_PIN);
    report(b"green", GPIOE, GREEN_PIN, false);

    // Blue.
    gpio_clear(GPIOB, 1 << BLUE_PIN);
    report(b"blue", GPIOB, BLUE_PIN, true);
    gpio_set(GPIOB, 1 << BLUE_PIN);
    report(b"blue", GPIOB, BLUE_PIN, false);

    // Leave red lit; the heartbeat reports all three actual pad levels.
    gpio_clear(GPIOB, 1 << RED_PIN);

    let mut heartbeat: u32 = 0;
    loop {
        puts(b"hb=");
        put_dec32(heartbeat);
        puts(b" red=");
        put_dec32(pin_level(GPIOB, RED_PIN));
        puts(b" green=");
        put_dec32(pin_level(GPIOE, GREEN_PIN));
        puts(b" blue=");
        put_dec32(pin_level(GPIOB, BLUE_PIN));
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
