// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! STM32F4-Discovery (MB997, STM32F407VGT6) smoke firmware: a running light
//! over the four user LEDs plus a heartbeat line on USART2, in bare registers.
//!
//! Board facts (UM1472; Zephyr `stm32f4_disco`):
//!   PD12..PD15 — user LEDs LD4 (green), LD3 (orange), LD5 (red), LD6 (blue),
//!                all active high (core configs/systems/stm32f4-discovery.yaml)
//!   PA0        — USER button (not driven here)
//!   PA2/PA3    — USART2 TX/RX at 115200 8N1 from the 16 MHz HSI. On this
//!                board the ST-LINK VCP is NOT wired to the MCU UART, so the
//!                console pins are read from the PA2/PA3 header pins.
//!
//! Each beat lights one LED, prints `hb=<n> <led>=on odr=<hex>`, switches it
//! off and prints `=off`. The `odr=` field is READ BACK from GPIOD_ODR after
//! the write, so a passing run reports the pad the firmware actually drove.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC (F4): AHB1ENR gates the GPIO ports, APB1ENR gates USART2.
const RCC: u32 = 0x4002_3800;
const RCC_AHB1ENR: u32 = RCC + 0x30;
const RCC_APB1ENR: u32 = RCC + 0x40;

// USART2 (F4 classic block: SR/DR).
const USART2: u32 = 0x4000_4400;
const USART_SR: *const u32 = USART2 as *const u32;
const USART_DR: *mut u32 = (USART2 + 0x04) as *mut u32;
const USART_BRR: *mut u32 = (USART2 + 0x08) as *mut u32;
const USART_CR1: *mut u32 = (USART2 + 0x0C) as *mut u32;
const TXE: u32 = 1 << 7;

const GPIOA: u32 = 0x4002_0000;
const GPIOD: u32 = 0x4002_0C00;

/// GPIO register offsets (STM32 GPIO v2 block: MODER/ODR/BSRR).
const MODER: u32 = 0x00;
const AFRL: u32 = 0x20;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;

/// User LEDs in running-light order: (console name, pin).
const LEDS: [(&[u8], u32); 4] = [(b"ld4", 12), (b"ld3", 13), (b"ld5", 14), (b"ld6", 15)];

fn reg_rmw(addr: u32, clear: u32, set: u32) {
    unsafe {
        let p = addr as *mut u32;
        write_volatile(p, (read_volatile(p) & !clear) | set);
    }
}

/// Route `pin` of the GPIO port at `base` to alternate function `af`.
fn pin_af(base: u32, pin: u32, af: u32) {
    reg_rmw(base + MODER, 0b11 << (pin * 2), 0b10 << (pin * 2));
    let (reg, shift) = if pin < 8 {
        (AFRL, pin * 4)
    } else {
        (0x24, (pin - 8) * 4) // AFRH
    };
    reg_rmw(base + reg, 0xF << shift, af << shift);
}

fn putc(b: u8) {
    unsafe {
        while read_volatile(USART_SR) & TXE == 0 {}
        write_volatile(USART_DR, b as u32);
    }
}

fn send(s: &[u8]) {
    for b in s {
        putc(*b);
    }
}

fn put_dec(mut v: u32) {
    let mut digits = [0u8; 10];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    while n > 0 {
        n -= 1;
        putc(digits[n]);
    }
}

fn put_hex32(v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        putc(HEX[((v >> (i * 4)) & 0xF) as usize]);
    }
}

fn led_drive(pin: u32, on: bool) {
    // BSRR: low half sets the pin, high half resets it.
    let bit = if on { 1 << pin } else { 1 << (pin + 16) };
    unsafe { write_volatile((GPIOD + BSRR) as *mut u32, bit) };
}

fn odr() -> u32 {
    unsafe { read_volatile((GPIOD + ODR) as *const u32) }
}

fn delay() {
    for _ in 0..200000u32 {
        let _ = odr();
    }
}

#[entry]
fn main() -> ! {
    // GPIOA (USART2 pins) + GPIOD (LEDs), then USART2.
    reg_rmw(RCC_AHB1ENR, 0, (1 << 0) | (1 << 3));
    reg_rmw(RCC_APB1ENR, 0, 1 << 17);
    pin_af(GPIOA, 2, 7);
    pin_af(GPIOA, 3, 7);
    unsafe {
        // HSI = 16 MHz: USARTDIV 8.6875 -> mantissa 8, fraction 11 -> 0x8B.
        write_volatile(USART_BRR, 0x8B);
        write_volatile(USART_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }
    for (_, pin) in LEDS {
        reg_rmw(GPIOD + MODER, 0b11 << (pin * 2), 0b01 << (pin * 2));
        led_drive(pin, false);
    }
    send(b"F4-DISCO SMOKE\r\n");

    let mut beat: u32 = 0;
    loop {
        let (name, pin) = LEDS[((beat / 2) as usize) % LEDS.len()];
        led_drive(pin, beat & 1 == 0);
        send(b"hb=");
        put_dec(beat);
        send(b" ");
        send(name);
        let lit = odr() & (1 << pin) != 0;
        send(if lit { b"=on odr=" } else { b"=off odr=" });
        put_hex32(odr());
        send(b"\r\n");
        beat = beat.wrapping_add(1);
        delay();
    }
}
