// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! NUCLEO-L552ZE-Q (STM32L552ZET6Q, Cortex-M33) smoke firmware: a running
//! light over the three user LEDs plus a heartbeat line on LPUART1, in bare
//! registers.
//!
//! Board facts (UM2581; Zephyr `nucleo_l552ze_q`):
//!   LD1 (green) PC7, LD2 (blue) PB7, LD3 (red) PA9 — active high
//!   B1  (user)  PC13 — active high
//!   VCP console LPUART1 PG7 (TX) / PG8 (RX), AF8, 115200 8N1
//!
//! The firmware enables the L5 clock gates (RCC_AHB2ENR for GPIOA/B/C/G,
//! RCC_APB1ENR2.LPUART1EN), enables VDDIO2 (PG[15:2] sit on the VDDIO2 rail
//! and stay isolated until PWR_CR2.IOSV is set), configures PG7/PG8 as AF8,
//! walks the LEDs and prints the actual GPIO ODR readback, then reads the
//! button and heartbeats.
//!
//! Only the reset MSI (4 MHz) is used: LPUART BRR = 256 * 4 MHz / 115200 =
//! 8889 = 0x22B9 (the LPUART uses a 256x oversampling divider).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC (RM0438 RCC chapter): AHB2ENR @ 0x4C (GPIO), APB1ENR1 @ 0x58 (PWR),
// APB1ENR2 @ 0x5C (LPUART1).
const RCC: u32 = 0x4002_1000;
const RCC_AHB2ENR: *mut u32 = (RCC + 0x4C) as *mut u32;
const RCC_APB1ENR1: *mut u32 = (RCC + 0x58) as *mut u32;
const RCC_APB1ENR2: *mut u32 = (RCC + 0x5C) as *mut u32;
const RCC_APB1ENR1_PWREN: u32 = 1 << 28;

// PWR_CR2 @ 0x04: IOSV (bit 9) removes the VDDIO2 isolation on PG[15:2]
// (CMSIS `PWR_CR2_IOSV_Pos = 9`; HAL_PWREx_EnableVddIO2()).
const PWR_CR2: *mut u32 = (0x4000_7000 + 0x04) as *mut u32;
const PWR_CR2_IOSV: u32 = 1 << 9;

// GPIO register offsets (STM32 GPIO v2 block).
const MODER: u32 = 0x00;
const IDR: u32 = 0x10;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;
const AFRL: u32 = 0x20;
const AFRH: u32 = 0x24;

const GPIOA: u32 = 0x4202_0000;
const GPIOB: u32 = 0x4202_0400;
const GPIOC: u32 = 0x4202_0800;
const GPIOG: u32 = 0x4202_1800;

// LPUART1 (USART v2 register layout): CR1 @ 0x00, BRR @ 0x0C, ISR @ 0x1C,
// TDR @ 0x28.
const LPUART1: u32 = 0x4000_8000;
const LPUART_CR1: *mut u32 = LPUART1 as *mut u32;
const LPUART_BRR: *mut u32 = (LPUART1 + 0x0C) as *mut u32;
const LPUART_ISR: *const u32 = (LPUART1 + 0x1C) as *const u32;
const LPUART_TDR: *mut u32 = (LPUART1 + 0x28) as *mut u32;

const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_RE: u32 = 1 << 2;
const USART_CR1_TE: u32 = 1 << 3;

#[derive(Clone, Copy)]
struct Led {
    name: &'static [u8],
    port: u32,
    pin: u32,
}

const LEDS: [Led; 3] = [
    Led {
        name: b"ld1",
        port: GPIOC,
        pin: 7,
    },
    Led {
        name: b"ld2",
        port: GPIOB,
        pin: 7,
    },
    Led {
        name: b"ld3",
        port: GPIOA,
        pin: 9,
    },
];

fn putc(byte: u8) {
    unsafe {
        while read_volatile(LPUART_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(LPUART_TDR, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn put_hex32(value: u32) {
    for shift in (0..8).rev() {
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
        (AFRH, (pin - 8) * 4)
    };
    reg_rmw(base + reg, 0xF << shift, af << shift);
}

fn led_drive(led: Led, on: bool) {
    let bit = if on {
        1 << led.pin
    } else {
        1 << (led.pin + 16)
    };
    unsafe { write_volatile((led.port + BSRR) as *mut u32, bit) };
}

fn report(on: bool, led: Led) {
    puts(b"led=");
    puts(led.name);
    puts(if on { b" odr=" } else { b"_off odr=" });
    put_hex32(unsafe { read_volatile((led.port + ODR) as *const u32) });
    putc(b'\n');
}

fn button_level() -> u32 {
    unsafe { (read_volatile((GPIOC + IDR) as *const u32) >> 13) & 1 }
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
        // GPIOA (LD3), GPIOB (LD2), GPIOC (LD1 + button), GPIOG (LPUART1).
        write_volatile(
            RCC_AHB2ENR,
            read_volatile(RCC_AHB2ENR) | (1 << 0) | (1 << 1) | (1 << 2) | (1 << 6),
        );
        // LPUART1EN: APB1ENR2 bit 0.
        write_volatile(RCC_APB1ENR2, read_volatile(RCC_APB1ENR2) | 1);

        // PG7/PG8 are VDDIO2 pins: clock the PWR block, then set IOSV so
        // the port G pads are no longer isolated. Without this the VCP
        // stays silent on the real board.
        write_volatile(
            RCC_APB1ENR1,
            read_volatile(RCC_APB1ENR1) | RCC_APB1ENR1_PWREN,
        );
        write_volatile(PWR_CR2, read_volatile(PWR_CR2) | PWR_CR2_IOSV);

        // PG7/PG8 as AF8 (LPUART1_TX/RX).
        pin_af(GPIOG, 7, 8);
        pin_af(GPIOG, 8, 8);

        // The three LEDs as push-pull outputs, initially low.
        for led in LEDS {
            reg_rmw(
                led.port + MODER,
                0b11 << (led.pin * 2),
                0b01 << (led.pin * 2),
            );
            led_drive(led, false);
        }
        // PC13 as input (B1).
        let moder = (GPIOC + MODER) as *mut u32;
        write_volatile(moder, read_volatile(moder) & !(0b11 << 26));

        // MSI 4 MHz / 115200 with the LPUART 256x divider.
        write_volatile(LPUART_BRR, 0x22B9);
        write_volatile(LPUART_CR1, USART_CR1_UE | USART_CR1_TE | USART_CR1_RE);
    }

    puts(b"NUCLEO-L552ZE-Q SMOKE\n");
    puts(b"cpu=cortex-m33 uart=lpuart1\n");

    for led in LEDS {
        led_drive(led, true);
        report(true, led);
        led_drive(led, false);
        report(false, led);
    }

    // Leave LD1 lit so the heartbeat's ODR readback is non-zero.
    led_drive(LEDS[0], true);

    puts(b"btn=");
    put_dec32(button_level());
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        puts(b"hb=");
        put_dec32(heartbeat);
        puts(b" odr=");
        put_hex32(unsafe { read_volatile((GPIOC + ODR) as *const u32) });
        puts(b" btn=");
        put_dec32(button_level());
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
