// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! Apollo4 Plus EVB (AMAP42KP-KBR, Cortex-M4F) L1 smoke: UART0 console, a walk
//! over the three user LEDs with RD readback, the two buttons' pad levels and
//! a heartbeat, in bare registers.
//!
//! Board facts (Zephyr `boards/ambiq/apollo4p_evb`, apollo4p_evb.dts and
//! apollo4p_evb-pinctrl.dtsi):
//!   LED0 GPIO30, LED1 GPIO90, LED2 GPIO97 (active low)
//!   BTN0 GPIO18, BTN1 GPIO19 (active low, pull-up)
//!   UART0 TX GPIO60 / RX GPIO47 (FNCSEL 4), J-Link VCOM, 115200 8N1
//!
//! Registers: Ambiq CMSIS `apollo4p.h` (GPIO_Type @ 0x4001_0000,
//! PWRCTRL_Type @ 0x4002_1000, UART0_Type @ 0x4001_C000).
//!
//! Clock: the core stays in the reset LP mode (MCUPERFREQ status LP,
//! 96 MHz). UART0 takes its clock from CR.CLKSEL; the smoke selects the
//! 24-MHz source (CLKSEL = 1) and derives the divisor from it:
//!   BRD  = 24 000 000 / (16 × 115 200) = 13.0208
//!   IBRD = 13, FBRD = round(0.0208 × 64) = 1
//! computed at run time as round(64 × BRD) = 833 = 13 × 64 + 1 (the same pair
//! AmbiqSuite's am_hal_uart config_baud produces). Achieved 115 384 baud
//! (+0.16 %).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── PWRCTRL (apollo4p.h PWRCTRL_Type, base 0x4002_1000) ─────────────────────
const PWRCTRL: u32 = 0x4002_1000;
const PWRCTRL_MCUPERFREQ: u32 = PWRCTRL; // + 0x00
const PWRCTRL_DEVPWREN: u32 = PWRCTRL + 0x04;
const PWRCTRL_DEVPWRSTATUS: u32 = PWRCTRL + 0x08;
/// MCUPERFREQ.MCUPERFSTATUS [4:3]: 0 ULP 24 MHz, 1 LP 96 MHz, 2 HP 192 MHz.
const MCUPERFSTATUS_SHIFT: u32 = 3;
/// DEVPWREN.PWRENUART0 / DEVPWRSTATUS.PWRSTUART0 (bit 9).
const PWR_UART0: u32 = 1 << 9;

// ── GPIO (apollo4p.h GPIO_Type, base 0x4001_0000) ───────────────────────────
const GPIO: u32 = 0x4001_0000;
const GPIO_PADKEY: u32 = GPIO + 0x200;
const GPIO_RD0: u32 = GPIO + 0x204;
const GPIO_WT0: u32 = GPIO + 0x214;
const GPIO_WTS0: u32 = GPIO + 0x224;
const GPIO_WTC0: u32 = GPIO + 0x234;
/// GPIO_PADKEY_PADKEY_Key.
const PADKEY_KEY: u32 = 0x73;
/// PINCFGn fields.
const FNCSEL_GPIO: u32 = 3;
/// FNCSEL 4 on GPIO60 = UART0TX, on GPIO47 = UART0RX.
const FNCSEL_UART0: u32 = 4;
const INPEN: u32 = 1 << 4;
const OUTCFG_PUSHPULL: u32 = 1 << 8;
/// PULLCFG [15:13] = 5: 24K pull-up.
const PULLCFG_PU24K: u32 = 5 << 13;

const LED0: u32 = 30;
const LED1: u32 = 90;
const LED2: u32 = 97;
const BTN0: u32 = 18;
const BTN1: u32 = 19;
const UART0_TX: u32 = 60;
const UART0_RX: u32 = 47;

// ── UART0 (apollo4p.h UART0_Type, base 0x4001_C000; PL011 layout) ───────────
const UART0: u32 = 0x4001_C000;
const UART_DR: u32 = UART0;
const UART_FR: u32 = UART0 + 0x18;
const UART_IBRD: u32 = UART0 + 0x24;
const UART_FBRD: u32 = UART0 + 0x28;
const UART_LCRH: u32 = UART0 + 0x2C;
const UART_CR: u32 = UART0 + 0x30;
/// FR.TXFF (bit 5).
const UART_FR_TXFF: u32 = 1 << 5;
/// LCRH.WLEN [6:5] = 3 (8 bits), no parity, one stop, FIFOs off.
const UART_LCRH_8N1: u32 = 0x3 << 5;
/// CR: UARTEN [0], CLKEN [3], CLKSEL [6:4], TXE [8], RXE [9].
const UART_CR_UARTEN: u32 = 1 << 0;
const UART_CR_CLKEN: u32 = 1 << 3;
const UART_CR_CLKSEL_SHIFT: u32 = 4;
const UART_CR_CLKSEL_24MHZ: u32 = 1;
const UART_CR_TXE: u32 = 1 << 8;
const UART_CR_RXE: u32 = 1 << 9;

const BAUD: u32 = 115_200;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn pincfg(pin: u32) -> u32 {
    GPIO + 4 * pin
}

fn bank(base: u32, pin: u32) -> u32 {
    base + 4 * (pin / 32)
}

fn mask(pin: u32) -> u32 {
    1 << (pin % 32)
}

/// The pad level as RD reports it (INPEN is set on every pin read here).
fn pin(n: u32) -> u32 {
    (rd(bank(GPIO_RD0, n)) >> (n % 32)) & 1
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

/// UART clock for CR.CLKSEL 1..4: 24, 12, 6, 3 MHz (0 = no clock).
fn uart_clock_hz(clksel: u32) -> u32 {
    match clksel {
        1..=4 => 24_000_000 >> (clksel - 1),
        _ => 0,
    }
}

/// The three LEDs as an active-high mask (bit 0 = LED0), read through RD.
fn leds_lit() -> u32 {
    [LED0, LED1, LED2]
        .iter()
        .enumerate()
        .fold(0, |m, (i, &n)| m | ((pin(n) ^ 1) << i))
}

fn led_set(n: u32, on: bool) {
    // Active low: WTC lights the LED, WTS turns it off. Single-bit strobes,
    // so no other pin of the bank changes.
    wr(bank(if on { GPIO_WTC0 } else { GPIO_WTS0 }, n), mask(n));
}

fn report(name: &[u8], n: u32, on: bool) {
    puts(b"led=");
    puts(name);
    puts(if on { b" on rd=" } else { b" off rd=" });
    put_dec32(pin(n));
    puts(b" lit=");
    put_dec32(leds_lit());
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
    // 1. Power the UART0 domain and wait for its status bit, as
    //    am_hal_pwrctrl_periph_enable does.
    wr(PWRCTRL_DEVPWREN, rd(PWRCTRL_DEVPWREN) | PWR_UART0);
    let mut polls = 0u32;
    while rd(PWRCTRL_DEVPWRSTATUS) & PWR_UART0 == 0 && polls < 1000 {
        polls += 1;
    }
    let uart0_powered = (rd(PWRCTRL_DEVPWRSTATUS) & PWR_UART0) >> 9;

    // 2. Pads: unlock, UART0 on GPIO60/47, LEDs off (high) before they
    //    become push-pull outputs, buttons as pulled-up inputs, relock.
    wr(GPIO_PADKEY, PADKEY_KEY);
    wr(pincfg(UART0_TX), FNCSEL_UART0);
    wr(pincfg(UART0_RX), FNCSEL_UART0 | INPEN);
    for n in [LED0, LED1, LED2] {
        wr(bank(GPIO_WTS0, n), mask(n));
        wr(pincfg(n), FNCSEL_GPIO | OUTCFG_PUSHPULL | INPEN);
    }
    for n in [BTN0, BTN1] {
        wr(pincfg(n), FNCSEL_GPIO | INPEN | PULLCFG_PU24K);
    }
    wr(GPIO_PADKEY, 0);

    // 3. UART0: clock select, divisors from that clock, LCRH, enable.
    wr(UART_CR, 0);
    wr(
        UART_CR,
        UART_CR_CLKEN | (UART_CR_CLKSEL_24MHZ << UART_CR_CLKSEL_SHIFT),
    );
    // The divisor follows the CLKSEL the firmware selected. (It is not read
    // back from CR: on silicon it would be, but the simulator's PL011 model
    // does not store CR — see examples/apollo4p-evb/VALIDATION.md.)
    let clksel = UART_CR_CLKSEL_24MHZ;
    let clk = uart_clock_hz(clksel);
    let brd64 = (clk * 8 / BAUD).div_ceil(2); // round(64 × clk / (16 × baud))
    wr(UART_IBRD, brd64 / 64);
    wr(UART_FBRD, brd64 % 64);
    wr(UART_LCRH, UART_LCRH_8N1);
    wr(
        UART_CR,
        UART_CR_CLKEN
            | (clksel << UART_CR_CLKSEL_SHIFT)
            | UART_CR_UARTEN
            | UART_CR_TXE
            | UART_CR_RXE,
    );

    puts(b"APOLLO4P EVB SMOKE\n");
    puts(b"cpu=cortex-m4f uart=uart0\n");
    puts(b"pwr perf=");
    puts(
        match (rd(PWRCTRL_MCUPERFREQ) >> MCUPERFSTATUS_SHIFT) & 0x3 {
            0 => b"ulp",
            1 => b"lp",
            2 => b"hp",
            _ => b"rsvd",
        },
    );
    puts(b" uart0_pwr=");
    put_dec32(uart0_powered);
    putc(b'\n');
    puts(b"uart0 clksel=");
    put_dec32(clksel);
    puts(b" clk=");
    put_dec32(clk);
    puts(b" ibrd=");
    put_dec32(rd(UART_IBRD));
    puts(b" fbrd=");
    put_dec32(rd(UART_FBRD));
    putc(b'\n');

    for (name, n) in [
        (&b"led0"[..], LED0),
        (&b"led1"[..], LED1),
        (&b"led2"[..], LED2),
    ] {
        led_set(n, true);
        report(name, n, true);
        led_set(n, false);
        report(name, n, false);
    }

    // The WT latch of each LED bank: the strobes left exactly the LED bits
    // high (off) and nothing else set.
    puts(b"wt0=");
    put_dec32(u32::from(rd(bank(GPIO_WT0, LED0)) == mask(LED0)));
    puts(b" wt2=");
    put_dec32(u32::from(rd(bank(GPIO_WT0, LED1)) == mask(LED1)));
    puts(b" wt3=");
    put_dec32(u32::from(rd(bank(GPIO_WT0, LED2)) == mask(LED2)));
    putc(b'\n');

    // Buttons are active low on the 24K pull-up: released reads 1.
    puts(b"btn0=");
    put_dec32(pin(BTN0));
    puts(b" btn1=");
    put_dec32(pin(BTN1));
    putc(b'\n');

    // Leave LED0 lit; the heartbeat reports the three LED pad levels.
    led_set(LED0, true);

    let mut heartbeat: u32 = 0;
    loop {
        puts(b"hb=");
        put_dec32(heartbeat);
        puts(b" led0=");
        put_dec32(pin(LED0));
        puts(b" led1=");
        put_dec32(pin(LED1));
        puts(b" led2=");
        put_dec32(pin(LED2));
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
