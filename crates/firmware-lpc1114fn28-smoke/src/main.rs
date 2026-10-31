// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP LPC1114FN28/102 (DIP-28, Cortex-M0) smoke firmware.
//!
//! Bring-up order, as NXP's CMSIS `system_LPC11xx.c` and mbed-os
//! `TARGET_LPC11XX_11CXX/serial_api.c` do it:
//! 1. `SystemInit`: SYSPLLCLKSEL = IRC, SYSPLLCLKUEN 1/0/1 (poll bit 0),
//!    SYSPLLCTRL = 0x23 (M = 4, P = 2: 12 MHz IRC -> 48 MHz), power the PLL up
//!    (PDRUNCFG bit 7 = 0) and wait for SYSPLLSTAT.LOCK, MAINCLKSEL = PLL out,
//!    MAINCLKUEN 1/0/1, SYSAHBCLKDIV = 1, then the IOCON clock.
//! 2. GPIO and UART clocks (SYSAHBCLKCTRL bits 6 and 12), UARTCLKDIV = 1.
//! 3. PIO1_6/PIO1_7 to RXD/TXD (IOCON FUNC = 1).
//! 4. UART 8N1: LCR.DLAB, DLM = 0, DLL = 17, FDR = 0xF8 (MULVAL 15,
//!    DIVADDVAL 8) for 115089 baud from 48 MHz (mbed's search for 115200),
//!    DLAB clear, FIFOs enabled and reset.
//! 5. LED on PIO0_7 (the 20 mA high-drive pad, LED1 on the mbed LPC1114FN28):
//!    driven through the address-masked GPIO0 DATA window and read back.
//! 6. GPIO0 interrupt: PIO0_7 rising-edge sense, IE, NVIC ISER bit 31; the
//!    pad sees its own output, so each 0 -> 1 toggle enters the handler,
//!    which clears the edge through GPIO0IC.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// SYSCON @ 0x40048000 (LPC11xx_v6a.svd).
const SYSCON: u32 = 0x4004_8000;
const SYSPLLCTRL: u32 = 0x008;
const SYSPLLSTAT: u32 = 0x00C;
const SYSPLLCLKSEL: u32 = 0x040;
const SYSPLLCLKUEN: u32 = 0x044;
const MAINCLKSEL: u32 = 0x070;
const MAINCLKUEN: u32 = 0x074;
const SYSAHBCLKDIV: u32 = 0x078;
const SYSAHBCLKCTRL: u32 = 0x080;
const UARTCLKDIV: u32 = 0x098;
const PDRUNCFG: u32 = 0x238;
const DEVICE_ID: u32 = 0x3F4;
const AHB_GPIO: u32 = 1 << 6;
const AHB_UART: u32 = 1 << 12;
const AHB_IOCON: u32 = 1 << 16;
const PD_SYSPLL: u32 = 1 << 7;

// IOCON @ 0x40044000.
const IOCON: u32 = 0x4004_4000;
const IOCON_PIO1_6: u32 = 0x0A4;
const IOCON_PIO1_7: u32 = 0x0A8;
const IOCON_PIO0_7: u32 = 0x050;
const IOCON_FUNC_MASK: u32 = 0x7;

// UART @ 0x40008000: 16C550 with a 4-byte stride.
const UART: u32 = 0x4000_8000;
const THR: u32 = 0x00;
const DLL: u32 = 0x00;
const DLM: u32 = 0x04;
const FCR: u32 = 0x08;
const LCR: u32 = 0x0C;
const LSR: u32 = 0x14;
const FDR: u32 = 0x28;
const TER: u32 = 0x30;
const LCR_8N1: u32 = 0x03;
const LCR_DLAB: u32 = 1 << 7;
const LSR_THRE: u32 = 1 << 5;
const UART_DL: u32 = 17;
const FDR_MULVAL: u32 = 15;
const FDR_DIVADDVAL: u32 = 8;
const MAIN_HZ: u32 = 48_000_000;

// GPIO0 @ 0x50000000.
const GPIO0: u32 = 0x5000_0000;
const GPIO_DATA: u32 = 0x3FFC;
const GPIO_DIR: u32 = 0x8000;
const GPIO_IEV: u32 = 0x800C;
const GPIO_IE: u32 = 0x8010;
const GPIO_RIS: u32 = 0x8014;
const GPIO_MIS: u32 = 0x8018;
const GPIO_IC: u32 = 0x801C;
const LED_BIT: u32 = 7;
const LED: u32 = 1 << LED_BIT;
/// GPIO0 port interrupt (NVIC line 31).
const IRQ_GPIO0: i16 = 31;
const NVIC_ISER: *mut u32 = 0xE000_E100 as *mut u32;

static EDGES: AtomicU32 = AtomicU32::new(0);

fn rd(base: u32, offset: u32) -> u32 {
    unsafe { read_volatile((base + offset) as *const u32) }
}

fn wr(base: u32, offset: u32, value: u32) {
    unsafe { write_volatile((base + offset) as *mut u32, value) }
}

/// Address of GPIO0 DATA through the pin mask `mask` (address bits 13:2).
fn masked(mask: u32) -> u32 {
    mask << 2
}

fn putc(byte: u8) {
    while rd(UART, LSR) & LSR_THRE == 0 {}
    wr(UART, THR, byte as u32);
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

/// Write 1, 0, 1 to a clock-source update register and wait for bit 0.
fn clock_update(uen: u32) {
    wr(SYSCON, uen, 1);
    wr(SYSCON, uen, 0);
    wr(SYSCON, uen, 1);
    while rd(SYSCON, uen) & 1 == 0 {}
}

/// CMSIS `SystemInit` with the mbed LPC1114 settings (IRC -> PLL 48 MHz).
fn system_init() {
    wr(SYSCON, SYSPLLCLKSEL, 0);
    clock_update(SYSPLLCLKUEN);
    wr(SYSCON, SYSPLLCTRL, 0x23);
    wr(SYSCON, PDRUNCFG, rd(SYSCON, PDRUNCFG) & !PD_SYSPLL);
    while rd(SYSCON, SYSPLLSTAT) & 1 == 0 {}
    wr(SYSCON, MAINCLKSEL, 3);
    clock_update(MAINCLKUEN);
    wr(SYSCON, SYSAHBCLKDIV, 1);
    wr(SYSCON, SYSAHBCLKCTRL, rd(SYSCON, SYSAHBCLKCTRL) | AHB_IOCON);
}

fn uart_init() {
    wr(SYSCON, SYSAHBCLKCTRL, rd(SYSCON, SYSAHBCLKCTRL) | AHB_UART);
    wr(SYSCON, UARTCLKDIV, 1);
    for pin in [IOCON_PIO1_6, IOCON_PIO1_7] {
        wr(IOCON, pin, (rd(IOCON, pin) & !IOCON_FUNC_MASK) | 1);
    }
    wr(UART, LCR, LCR_8N1 | LCR_DLAB);
    wr(UART, DLM, UART_DL >> 8);
    wr(UART, DLL, UART_DL & 0xFF);
    wr(UART, FDR, (FDR_MULVAL << 4) | FDR_DIVADDVAL);
    wr(UART, LCR, LCR_8N1);
    // FIFO enable + RX/TX FIFO reset.
    wr(UART, FCR, 0x07);
}

fn report_led(label: &str) {
    puts(label);
    puts(" data=");
    put_hex(rd(GPIO0, GPIO_DATA), 3);
    puts(" led=");
    put_dec32(rd(GPIO0, masked(LED)) >> LED_BIT);
    puts(" ris=");
    put_hex(rd(GPIO0, GPIO_RIS), 3);
    putc(b'\n');
}

#[entry]
fn main() -> ! {
    system_init();
    uart_init();

    puts("LPC1114FN28 SMOKE\n");
    puts("cpu=cortex-m0 uart=uart pins=pio1_6/pio1_7 led=pio0_7\n");
    puts("device_id=");
    put_hex(rd(SYSCON, DEVICE_ID), 8);
    putc(b'\n');
    puts("clk pllctrl=");
    put_hex(rd(SYSCON, SYSPLLCTRL), 2);
    puts(" lock=");
    put_dec32(rd(SYSCON, SYSPLLSTAT) & 1);
    puts(" mainclksel=");
    put_dec32(rd(SYSCON, MAINCLKSEL));
    puts(" ahbclkctrl=");
    put_hex(rd(SYSCON, SYSAHBCLKCTRL), 5);
    putc(b'\n');

    // Read the divisor back through DLAB, as a driver verifying it would.
    wr(UART, LCR, LCR_8N1 | LCR_DLAB);
    let dl = (rd(UART, DLM) << 8) | rd(UART, DLL);
    wr(UART, LCR, LCR_8N1);
    let fdr = rd(UART, FDR);
    let mul = fdr >> 4;
    let add = fdr & 0xF;
    puts("uart lcr=");
    put_hex(rd(UART, LCR), 2);
    puts(" dl=");
    put_dec32(dl);
    puts(" fdr=");
    put_hex(fdr, 2);
    puts(" ter=");
    put_hex(rd(UART, TER), 2);
    puts(" baud=");
    put_dec32(MAIN_HZ / 16 * mul / (dl * (mul + add)));
    putc(b'\n');

    // LED: PIO0_7 as GPIO output, driven low through its mask.
    wr(SYSCON, SYSAHBCLKCTRL, rd(SYSCON, SYSAHBCLKCTRL) | AHB_GPIO);
    wr(
        IOCON,
        IOCON_PIO0_7,
        rd(IOCON, IOCON_PIO0_7) & !IOCON_FUNC_MASK,
    );
    wr(GPIO0, masked(LED), 0);
    wr(GPIO0, GPIO_DIR, rd(GPIO0, GPIO_DIR) | LED);
    report_led("led=off");
    // A write through another pin's mask must not move PIO0_7.
    wr(GPIO0, masked(1 << 6), 0xFFF);
    report_led("mask6");
    wr(GPIO0, masked(LED), 0xFFF);
    report_led("led=on");

    // Rising-edge interrupt on PIO0_7.
    wr(GPIO0, masked(LED), 0);
    wr(GPIO0, GPIO_IC, LED);
    wr(GPIO0, GPIO_IEV, LED);
    wr(GPIO0, GPIO_IE, LED);
    unsafe { write_volatile(NVIC_ISER, 1 << IRQ_GPIO0) };
    for _ in 0..3 {
        wr(GPIO0, masked(LED), LED);
        wr(GPIO0, masked(LED), 0);
    }
    puts("irq gpio0 edges=");
    put_dec32(EDGES.load(Ordering::Relaxed));
    puts(" mis=");
    put_hex(rd(GPIO0, GPIO_MIS), 3);
    putc(b'\n');
    wr(GPIO0, GPIO_IE, 0);

    let mut heartbeat: u32 = 0;
    loop {
        wr(GPIO0, masked(LED), rd(GPIO0, masked(LED)) ^ LED);
        puts("hb=");
        put_dec32(heartbeat);
        puts(" led=");
        put_dec32(rd(GPIO0, masked(LED)) >> LED_BIT);
        putc(b'\n');
        spin(100_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}

/// Every device interrupt lands here (no PAC). GPIO0 counts the edge and
/// acknowledges it; anything else is unexpected and parks the core.
#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == IRQ_GPIO0 {
        // ARMv6-M has no atomic read-modify-write; the handler is the only
        // writer, so a load/store pair is enough.
        EDGES.store(EDGES.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        wr(GPIO0, GPIO_IC, LED);
    } else {
        loop {}
    }
}
