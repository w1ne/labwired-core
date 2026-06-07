//! nRF52832 Tier-1 fixture firmware.
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
//! The nRF52832 chip YAML declares only `uart0`; all other rubric classes
//! (clock, gpio, timer, dma, irq) are not wired in the YAML and will resolve
//! to `na` by the parser — no explicit FAIL lines are emitted for them.
//!
//! Register offsets follow the nRF52832 Product Specification v1.4
//! (compatible with nRF52840 PS v1.7 for the shared UART0 layout).

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// ── UART0 base address (nRF52832 PS §15.2, §15.8 memory map) ─────────────
//
// Peripheral base: 0x40002000 (matches configs/chips/nrf52832.yaml).
// TXD register: offset 0x51C (PS §15.8.22, nRF52840 §35.10.25).
// ENABLE register: offset 0x500 (PS §15.8.20), value 4 = enable.
const UART0_BASE: u32 = 0x4000_2000;
const UART0_ENABLE: u32 = UART0_BASE + 0x500;
const UART0_TXD: u32 = UART0_BASE + 0x51C;

#[inline(always)]
fn reg_write(addr: u32, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

// ── UART0 output (raw register writes) ───────────────────────────────────
//
// Write each byte directly to the TXD register. The simulator's UART model
// captures each word write as one TX byte (low 8 bits). No FIFO polling
// needed — the sim model accepts writes unconditionally.
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

#[entry]
fn main() -> ! {
    // Enable UART0 (value 4 per Nordic PS UART.ENABLE field).
    reg_write(UART0_ENABLE, 4);

    // uart class: implicit — receiving "TIER1 done" proves UART is alive.
    // All other rubric classes (clock, gpio, timer, dma, irq) are not
    // declared in configs/chips/nrf52832.yaml and will resolve to na.

    uart_write_line("TIER1 done");

    loop {
        core::hint::spin_loop();
    }
}
