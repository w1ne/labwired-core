//! RP2040 Tier-1 fixture firmware.
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
//! The RP2040 chip YAML declares `uart0` (pl011 profile) and `pio0`.
//! `pio0` is not in CLASS_MARKERS so it does not declare any additional
//! rubric class. All rubric classes except `uart` resolve to `na`.
//!
//! Register offsets follow the RP2040 datasheet §4.2 (UART), which is an
//! ARM PrimeCell PL011: the data register (UARTDR) sits at offset 0x00.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// ── UART0 (RP2040 datasheet §4.2, base 0x40034000) ────────────────────────
//
// The simulator wires uart0 with profile "pl011" (ARM PrimeCell PL011, the
// RP2040's actual UART IP). In that layout the data register (UARTDR) sits at
// offset 0x00 — writing a byte here enqueues it for transmission. No CR1
// enable required in the simulator model — writes are captured unconditionally.
// (The earlier 0x28 offset assumed an stm32v2 profile; the chip YAML now uses
// pl011, so DR moved to 0x00.)
const UART0_BASE: u32 = 0x4003_4000;
const UART0_TDR: u32 = UART0_BASE + 0x00;

#[inline(always)]
fn reg_write(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

// ── UART0 output (raw register writes) ───────────────────────────────────
fn uart_write_byte(byte: u8) {
    reg_write(UART0_TDR, byte as u32);
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

#[entry]
fn main() -> ! {
    // uart: implicit via TIER1 done — no explicit line needed.
    // clock/gpio/timer/dma/irq: not declared in configs/chips/rp2040.yaml
    // → will resolve to na by the parser; no FAIL lines emitted.
    // (pio0 is declared but has no CLASS_MARKERS entry, so no extra class.)

    uart_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
