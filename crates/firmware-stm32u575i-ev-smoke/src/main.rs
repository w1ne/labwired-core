// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32U575I-EV (MB1550, STM32U575AII6Q) smoke firmware.
//!
//! Board facts (UM2854 Rev 2; ST STM32CubeU5 v1.9.0 `stm32u575i-ev-bsp`
//! v1.4.1 `stm32u575i_eval.h`):
//!   LED5 (green) PB7 — active high (`BSP_LED_On` drives the pin high)
//!   LED6 (red)   PH7 — active high
//!   USER button  PC13 — active high, external pull-down
//!   COM1         USART1 PA9/PA10, AF7, 115200 8N1 (ST-LINK-V3E VCP)
//!
//! LED7/LED8 sit behind the MFX I/O expander and are not driven here.
//!
//! The smoke enables the two GPIO port clocks and USART1, drives LED5 and LED6
//! independently, and prints the GPIODB/GPIOH ODR values it READ BACK after
//! each write. The `odr_*` fields are the pads the firmware actually drove, not
//! an assumed state.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC @ 0x4602_0C00: AHB2ENR1 (GPIO clocks) @ +0x8C, APB2ENR @ +0xA4.
const RCC_AHB2ENR1: *mut u32 = (0x4602_0C00 + 0x8C) as *mut u32;
const RCC_APB2ENR: *mut u32 = (0x4602_0C00 + 0xA4) as *mut u32;

const RCC_AHB2ENR1_GPIOBEN: u32 = 1 << 1;
const RCC_AHB2ENR1_GPIOHEN: u32 = 1 << 7;
const RCC_APB2ENR_USART1EN: u32 = 1 << 14;

// GPIO v2 block: MODER @ 0x00, ODR @ 0x14, BSRR @ 0x18.
const GPIOB: u32 = 0x4202_0400;
const GPIOH: u32 = 0x4202_1C00;
const MODER: u32 = 0x00;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;

// USART1 TDR @ 0x4001_3800 + 0x28 (USART v2 register map, RM0456).
const USART1_TDR: *mut u8 = (0x4001_3800 + 0x28) as *mut u8;

const LED5_PIN: u32 = 7; // PB7
const LED6_PIN: u32 = 7; // PH7

fn reg_rmw(addr: u32, clear: u32, set: u32) {
    unsafe {
        let p = addr as *mut u32;
        write_volatile(p, (read_volatile(p) & !clear) | set);
    }
}

fn putc(byte: u8) {
    unsafe { write_volatile(USART1_TDR, byte) };
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

fn put_hex32(value: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for shift in (0..8).rev() {
        putc(HEX[((value >> (shift * 4)) & 0xF) as usize]);
    }
}

fn put_dec32(mut value: u32) {
    let mut digits = [0u8; 10];
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
    }
}

fn drive(port: u32, pin: u32, on: bool) {
    // BSRR: low half sets, high half resets.
    let bit = if on { 1 << pin } else { 1 << (pin + 16) };
    unsafe { write_volatile((port + BSRR) as *mut u32, bit) };
}

fn odr(port: u32) -> u32 {
    unsafe { read_volatile((port + ODR) as *const u32) }
}

fn report(label: &str) {
    puts(label);
    puts(" odr_b=");
    put_hex32(odr(GPIOB));
    puts(" odr_h=");
    put_hex32(odr(GPIOH));
    putc(b'\n');
}

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
        write_volatile(
            RCC_AHB2ENR1,
            read_volatile(RCC_AHB2ENR1) | RCC_AHB2ENR1_GPIOBEN | RCC_AHB2ENR1_GPIOHEN,
        );
        write_volatile(
            RCC_APB2ENR,
            read_volatile(RCC_APB2ENR) | RCC_APB2ENR_USART1EN,
        );
    }

    // Both user LEDs are plain push-pull outputs (MODER = 01).
    reg_rmw(
        GPIOB + MODER,
        0b11 << (LED5_PIN * 2),
        0b01 << (LED5_PIN * 2),
    );
    reg_rmw(
        GPIOH + MODER,
        0b11 << (LED6_PIN * 2),
        0b01 << (LED6_PIN * 2),
    );
    drive(GPIOB, LED5_PIN, false);
    drive(GPIOH, LED6_PIN, false);

    puts("STM32U575I-EV SMOKE\n");
    puts("cpu=cortex-m33 uart=usart1\n");

    report("leds=off");
    drive(GPIOB, LED5_PIN, true);
    report("led5=on");
    drive(GPIOH, LED6_PIN, true);
    report("led6=on");
    drive(GPIOB, LED5_PIN, false);
    report("led5=off");
    drive(GPIOH, LED6_PIN, false);
    report("led6=off");

    let mut heartbeat: u32 = 0;
    loop {
        let on = heartbeat & 1 == 0;
        drive(GPIOB, LED5_PIN, on);
        drive(GPIOH, LED6_PIN, on);
        puts("hb=");
        put_dec32(heartbeat);
        report("");
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
