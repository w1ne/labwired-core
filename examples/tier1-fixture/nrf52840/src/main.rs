//! nRF52840 Tier-1 fixture firmware.
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses and reports one line per class over UART0 using the
//! TIER1 protocol:
//!
//! ```text
//! TIER1 <class> PASS
//! TIER1 <class> FAIL code=<reason>
//! TIER1 done
//! ```
//!
//! The `uart` class is implicit: receiving `TIER1 done` over UART0 is itself
//! the proof of a working UART path.
//!
//! The nRF52840 chip YAML declares `uart0`, `gpio0`, and `gpio1`; all other
//! rubric classes (clock, timer, dma, irq) are not wired in the YAML and
//! resolve to `na` by the parser.
//!
//! Register offsets follow the nRF52840 Product Specification v1.7 §6.10
//! (GPIO) and §35.10 (UART).

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// ── UART0 (nRF52840 PS §35.10, base 0x40002000) ───────────────────────────
//
// ENABLE offset 0x500, value 4 = enabled (PS §35.10.20).
// TXD    offset 0x51C — write a byte to transmit (PS §35.10.25).
const UART0_BASE: u32 = 0x4000_2000;
const UART0_ENABLE: u32 = UART0_BASE + 0x500;
const UART0_TXD: u32 = UART0_BASE + 0x51C;

// ── GPIO0 (nRF52840 PS §6.10, base 0x50000000) ────────────────────────────
//
// OUT     offset 0x504 — output register (read current output state).
// OUTSET  offset 0x508 — write 1 to set pins high.
// OUTCLR  offset 0x50C — write 1 to clear pins low.
// DIRSET  offset 0x518 — write 1 to configure pins as output.
//
// Pin 13 (P0.13) carries no boot-strap on nRF52840-DK; safe to toggle.
const GPIO0_BASE: u32 = 0x5000_0000;
const GPIO0_OUT: u32 = GPIO0_BASE + 0x504;
const GPIO0_OUTSET: u32 = GPIO0_BASE + 0x508;
const GPIO0_OUTCLR: u32 = GPIO0_BASE + 0x50C;
const GPIO0_DIRSET: u32 = GPIO0_BASE + 0x518;

#[inline(always)]
fn reg_read(addr: u32) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
fn reg_write(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

// ── UART0 output (raw register writes) ───────────────────────────────────
fn uart_write_byte(byte: u8) {
    reg_write(UART0_TXD, byte as u32);
}

fn uart_write_str(s: &str) {
    for b in s.as_bytes() {
        uart_write_byte(*b);
    }
}

fn uart_write_line(s: &str) {
    uart_write_str(s);
    uart_write_str("\r\n");
}

fn report(class: &str, result: Result<(), &'static str>) {
    uart_write_str("TIER1 ");
    uart_write_str(class);
    match result {
        Ok(()) => uart_write_line(" PASS"),
        Err(code) => {
            uart_write_str(" FAIL code=");
            uart_write_line(code);
        }
    }
}

// ── gpio: DIRSET + OUTSET/OUTCLR on P0.13, read back via OUT ─────────────
//
// nRF52840 PS §6.10.12 (OUTSET), §6.10.13 (OUTCLR), §6.10.11 (OUT),
// §6.10.15 (DIRSET). Pin 13 is a safe test pin (no boot strap conflict).
// Sequence: configure as output, set high, verify OUT bit, clear, verify.
fn check_gpio() -> Result<(), &'static str> {
    const PIN: u32 = 1 << 13;

    // Configure pin as output.
    reg_write(GPIO0_DIRSET, PIN);

    // Set pin high via OUTSET, read back via OUT.
    reg_write(GPIO0_OUTSET, PIN);
    if reg_read(GPIO0_OUT) & PIN == 0 {
        return Err("gpio-out-outset");
    }

    // Clear pin via OUTCLR, read back via OUT.
    reg_write(GPIO0_OUTCLR, PIN);
    if reg_read(GPIO0_OUT) & PIN != 0 {
        return Err("gpio-out-outclr");
    }

    Ok(())
}

#[entry]
fn main() -> ! {
    // Enable UART0 (value 4 per Nordic PS UART.ENABLE field).
    reg_write(UART0_ENABLE, 4);

    // gpio: declared in chip YAML (gpio0 + gpio1); test GPIO0.
    report("gpio", check_gpio());

    // uart: implicit via TIER1 done — no explicit line needed.
    // clock/timer/dma/irq: not declared in configs/chips/nrf52840.yaml
    // → will resolve to na by the parser; no FAIL lines emitted.

    uart_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
