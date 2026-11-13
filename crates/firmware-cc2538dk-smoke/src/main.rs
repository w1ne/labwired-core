// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! CC2538DK (CC2538SF53, Cortex-M3) L1 smoke: UART0 console, a walk over the
//! SmartRF06 LEDs with GPIO_DATA readback, the keys' pad levels and a
//! heartbeat, in bare registers.
//!
//! Board facts (Contiki-NG `cc2538dk` dev/board.h; RIOT `cc2538dk`):
//!   LED1 red    PC0 (active high) — shared with the USB D+ pull-up, NOT driven
//!   LED2 yellow PC1 (active high)
//!   LED3 green  PC2 (active high)
//!   LED4 orange PC3 (active high)
//!   keys SELECT PA3, LEFT PC4, RIGHT PC5, UP PC6, DOWN PC7 (active low)
//!   UART0 PA0 (RX) / PA1 (TX) through the IOC, 115200 8N1
//!
//! Clock: the firmware stays on the reset clock — the 16-MHz HF-RC oscillator
//! with SYS_DIV = IO_DIV = 001 (SWRU319C Table 7-3: CLOCK_CTRL reset
//! 0x0103_0109, CLOCK_STA reset 0x0101_0109). It reads CLOCK_STA to learn the
//! system clock rather than assuming it.
//!
//! Baud (SWRU319C §18.4.2): UART_CC = 0 selects the SYS_DIV clock, HSE = 0
//! divides by 16, so
//!   BRD   = UARTSysClk / (16 × baud) = 16 000 000 / (16 × 115 200) = 8.6806
//!   IBRD  = 8
//!   FBRD  = integer(0.6806 × 64 + 0.5) = integer(44.06) = 44
//! The code computes the same pair at run time from CLOCK_STA as
//! round(64 × BRD) = round(4 × clk / baud) = 556 = 8 × 64 + 44.
//! Achieved: 16 MHz / (16 × (8 + 44/64)) = 115 108 baud (−0.08 %).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── SYS_CTRL (SWRU319C Table 7-3, base 0x400D_2000) ─────────────────────────
const SYS_CTRL: u32 = 0x400D_2000;
const SYS_CTRL_CLOCK_STA: u32 = SYS_CTRL + 0x04;
const SYS_CTRL_RCGCUART: u32 = SYS_CTRL + 0x28;
/// CLOCK_STA.OSC (bit 16): 1 = 16-MHz HF-RC, 0 = 32-MHz crystal.
const CLOCK_STA_OSC: u32 = 1 << 16;
/// CLOCK_STA.SYS_DIV (bits 2:0): 32 MHz >> SYS_DIV.
const CLOCK_STA_SYS_DIV: u32 = 0x7;
const CLOCK_STA_IO_DIV_SHIFT: u32 = 8;
/// RCGCUART.UART0 (bit 0).
const RCGCUART_UART0: u32 = 1 << 0;

// ── IOC (SWRU319C §9.1.1 / Table 9-7, base 0x400D_4000) ─────────────────────
const IOC: u32 = 0x400D_4000;
const IOC_PA1_SEL: u32 = IOC + 0x004;
const IOC_PA0_OVER: u32 = IOC + 0x080;
const IOC_PA1_OVER: u32 = IOC + 0x084;
const IOC_UARTRXD_UART0: u32 = IOC + 0x100;
/// Table 9-1: peripheral output select 0x0 = UART0 TXD.
const IOC_SEL_UART0_TXD: u32 = 0x0;
/// IOC_Pxx_OVER: 0x8 = output enable, 0x0 = no override (input, no pull).
const IOC_OVER_OE: u32 = 0x8;
const IOC_OVER_DIS: u32 = 0x0;
/// IOC_UARTRXD_UART0.INPUT_SEL: 0 = PA0.
const IOC_INPUT_PA0: u32 = 0;

// ── GPIO (SWRU319C Table 9-2; ports at Table 4-1 bases) ─────────────────────
const GPIO_A: u32 = 0x400D_9000;
const GPIO_C: u32 = 0x400D_B000;
const GPIO_DIR: u32 = 0x400;
const GPIO_AFSEL: u32 = 0x420;

const LED_YELLOW: u32 = 1; // PC1
const LED_GREEN: u32 = 2; // PC2
const LED_ORANGE: u32 = 3; // PC3
/// The LEDs the smoke drives (PC1..PC3). PC0 (LED1) is left alone.
const LED_MASK: u32 = (1 << LED_YELLOW) | (1 << LED_GREEN) | (1 << LED_ORANGE);

const KEY_SELECT: u32 = 3; // PA3
const KEY_LEFT: u32 = 4; // PC4
const KEY_RIGHT: u32 = 5; // PC5
const KEY_UP: u32 = 6; // PC6
const KEY_DOWN: u32 = 7; // PC7

// ── UART0 (SWRU319C Table 18-4, base 0x4000_C000) ───────────────────────────
const UART0: u32 = 0x4000_C000;
const UART_DR: u32 = UART0;
const UART_FR: u32 = UART0 + 0x018;
const UART_IBRD: u32 = UART0 + 0x024;
const UART_FBRD: u32 = UART0 + 0x028;
const UART_LCRH: u32 = UART0 + 0x02C;
const UART_CTL: u32 = UART0 + 0x030;
const UART_CC: u32 = UART0 + 0xFC8;
/// FR.TXFF (bit 5): transmit FIFO full.
const UART_FR_TXFF: u32 = 1 << 5;
/// LCRH.WLEN = 0x3 (8 bits), no parity, one stop, FIFOs off: 0x60 (§18.5).
const UART_LCRH_8N1: u32 = 0x3 << 5;
/// CTL: UARTEN (0), TXE (8), RXE (9).
const UART_CTL_EN: u32 = (1 << 0) | (1 << 8) | (1 << 9);
/// UART_CC.CS = 0: baud clock from the SYS_DIV setting.
const UART_CC_SYSDIV: u32 = 0x0;

const BAUD: u32 = 115_200;

/// Customer Configuration Area (SWRU319C §8.6, Tables 8-3 to 8-6), placed at
/// 0x0027_FFD4 by memory.x. The ROM boot code reads it at reset: a valid image
/// (word 1 == 0) whose vector table is at 0x0020_0000 (word 2) is entered
/// directly. Word 0's top byte is the backdoor configuration: 0xF3 = reserved
/// ones, backdoor enabled, active low, pad A pin 3 (the SELECT key — the RIOT
/// `cc2538dk` choice). The upper 32 bytes leave every page and debug unlocked.
/// The simulator fast-boots the ELF and does not run the ROM; the CCA is here
/// so the same image would boot on silicon.
#[link_section = ".flashcca"]
#[used]
static CCA: [u32; 11] = [
    0xF3FF_FFFF, // bytes 2004-2007: backdoor config (byte 2007 = 0xF3)
    0x0000_0000, // bytes 2008-2011: image valid
    0x0020_0000, // bytes 2012-2015: vector table address
    0xFFFF_FFFF, // bytes 2016-2047: page lock bits + LOCK_DEBUG_N
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
];

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// GPIO_DATA through the address mask: offset bits [9:2] select the pins a
/// read returns or a write alters (SWRU319C §9.2.2.1.2).
fn data_addr(port: u32, mask: u32) -> u32 {
    port + ((mask & 0xFF) << 2)
}

fn putc(byte: u8) {
    while rd(UART_FR) & UART_FR_TXFF != 0 {}
    wr(UART_DR, u32::from(byte));
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

fn put_hex8(value: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    putc(HEX[((value >> 4) & 0xF) as usize]);
    putc(HEX[(value & 0xF) as usize]);
}

/// System clock from CLOCK_STA: 32 MHz >> SYS_DIV.
fn system_clock_hz(clock_sta: u32) -> u32 {
    32_000_000 >> (clock_sta & CLOCK_STA_SYS_DIV)
}

/// One pin's level, read through its own DATA mask.
fn pin(port: u32, n: u32) -> u32 {
    (rd(data_addr(port, 1 << n)) >> n) & 1
}

fn led_set(n: u32, on: bool) {
    // A single masked store: only pin n changes (no read-modify-write).
    wr(data_addr(GPIO_C, 1 << n), if on { 0xFF } else { 0x00 });
}

fn report(name: &[u8], n: u32, on: bool) {
    puts(b"led=");
    puts(name);
    puts(if on { b" on pin=" } else { b" off pin=" });
    put_dec32(pin(GPIO_C, n));
    // The whole LED nibble through one masked read: proves the store above
    // touched only its own pin.
    puts(b" leds=");
    put_hex8(rd(data_addr(GPIO_C, LED_MASK)));
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
    // 1. Clock gate for UART0 (SWRU319C §18.5 step 1). GPIO has no gate.
    wr(SYS_CTRL_RCGCUART, rd(SYS_CTRL_RCGCUART) | RCGCUART_UART0);

    // 2-4. IOC: PA1 driven by UART0 TXD with the pad output enabled; PA0 an
    //      input routed to UART0 RX (§18.5 steps 2-4, §9.1.1).
    wr(IOC_PA1_SEL, IOC_SEL_UART0_TXD);
    wr(IOC_PA1_OVER, IOC_OVER_OE);
    wr(IOC_PA0_OVER, IOC_OVER_DIS);
    wr(IOC_UARTRXD_UART0, IOC_INPUT_PA0);
    // PA0/PA1 under peripheral (hardware) control (§9.2.2.3).
    wr(GPIO_A + GPIO_AFSEL, rd(GPIO_A + GPIO_AFSEL) | 0x03);

    // The baud divisor follows the clock CLOCK_STA reports, not an assumption.
    let clock_sta = rd(SYS_CTRL_CLOCK_STA);
    let sysclk = system_clock_hz(clock_sta);
    let brd64 = (sysclk * 8 / BAUD + 1) / 2; // round(64 × sysclk / (16 × baud))
    let ibrd = brd64 / 64;
    let fbrd = brd64 % 64;

    // UART0: disable, divisors, LCRH (latches the divisors), enable (§18.5).
    wr(UART_CTL, 0);
    wr(UART_CC, UART_CC_SYSDIV);
    wr(UART_IBRD, ibrd);
    wr(UART_FBRD, fbrd);
    wr(UART_LCRH, UART_LCRH_8N1);
    wr(UART_CTL, UART_CTL_EN);

    // LEDs PC1..PC3 as software-controlled outputs, off. PC0 is untouched.
    wr(GPIO_C + GPIO_AFSEL, rd(GPIO_C + GPIO_AFSEL) & !LED_MASK);
    wr(data_addr(GPIO_C, LED_MASK), 0);
    wr(GPIO_C + GPIO_DIR, rd(GPIO_C + GPIO_DIR) | LED_MASK);

    puts(b"CC2538DK SMOKE\n");
    puts(b"cpu=cortex-m3 uart=uart0\n");
    puts(b"clk osc=");
    puts(if clock_sta & CLOCK_STA_OSC != 0 {
        b"hfrc16"
    } else {
        b"xosc32"
    });
    puts(b" sys_div=");
    put_dec32(clock_sta & CLOCK_STA_SYS_DIV);
    puts(b" io_div=");
    put_dec32((clock_sta >> CLOCK_STA_IO_DIV_SHIFT) & 0x7);
    puts(b" sysclk=");
    put_dec32(sysclk);
    putc(b'\n');
    puts(b"uart0 rcgc=");
    put_dec32(rd(SYS_CTRL_RCGCUART) & RCGCUART_UART0);
    puts(b" ibrd=");
    put_dec32(rd(UART_IBRD));
    puts(b" fbrd=");
    put_dec32(rd(UART_FBRD));
    putc(b'\n');

    for (name, n) in [
        (&b"yellow"[..], LED_YELLOW),
        (&b"green"[..], LED_GREEN),
        (&b"orange"[..], LED_ORANGE),
    ] {
        led_set(n, true);
        report(name, n, true);
        led_set(n, false);
        report(name, n, false);
    }

    // Keys are active low with the reset IOC pull-up: released reads 1.
    puts(b"keys select=");
    put_dec32(pin(GPIO_A, KEY_SELECT));
    puts(b" left=");
    put_dec32(pin(GPIO_C, KEY_LEFT));
    puts(b" right=");
    put_dec32(pin(GPIO_C, KEY_RIGHT));
    puts(b" up=");
    put_dec32(pin(GPIO_C, KEY_UP));
    puts(b" down=");
    put_dec32(pin(GPIO_C, KEY_DOWN));
    putc(b'\n');

    // Leave yellow lit; the heartbeat reports the three driven pad levels.
    led_set(LED_YELLOW, true);

    let mut heartbeat: u32 = 0;
    loop {
        puts(b"hb=");
        put_dec32(heartbeat);
        puts(b" yellow=");
        put_dec32(pin(GPIO_C, LED_YELLOW));
        puts(b" green=");
        put_dec32(pin(GPIO_C, LED_GREEN));
        puts(b" orange=");
        put_dec32(pin(GPIO_C, LED_ORANGE));
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
