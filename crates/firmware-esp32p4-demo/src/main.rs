#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-P4 HP-core UART0 console smoke.
//!
//! Target: `riscv32imc-unknown-none-elf`. The P4 HP core is RV32IMAFC on
//! silicon (esp32p4.svd <cpu><name>RV32IMAFC</name>); this smoke needs no
//! atomics (A) and no hardware float (F), so it links for the `imc` subset the
//! engine's interpreter is exercised against. The F extension is NOT decoded by
//! the interpreter (crates/core/src/decoder/riscv.rs) — the image deliberately
//! avoids it, exactly as the C6 smoke avoids RV32A. The LP core and SMP are out
//! of scope.
//!
//! Bring-up: unlike the C6 there is **no PCR gate** on the P4 UARTs at L1 (the
//! P4 clock/reset block `HP_SYS_CLKRST` has a different layout and its tree is
//! not modelled), so the console path is just the divisor + the FIFO. Every
//! address below is the chip descriptor's fact, cited in
//! `configs/chips/esp32p4.yaml`.

use panic_halt as _;
use riscv_rt::entry;

/// UART0 FIFO, 32-bit write pushes the low byte onto the TX FIFO.
/// esp-idf esp32p4 `soc/uart_reg.h` (`UART_FIFO_REG`, offset 0x00), base
/// `DR_REG_UART_BASE` = 0x500C_A000 (esp32p4.svd UART0).
const UART0_FIFO: *mut u32 = 0x500C_A000 as *mut u32;

/// UART clock divider. Reset value 0x2B6 = 694 (esp32p4.svd
/// UART0.CLKDIV_SYNC, offset 0x14).
const UART0_CLKDIV: *mut u32 = 0x500C_A014 as *mut u32;

/// IO_MUX pad register for U0TXD. IO_MUX base `0x500E_1000` (esp32p4.svd
/// IO_MUX). Pad routing is NOT electrically enforced at L1 (the console shifts
/// bytes regardless), so this write is the honest bring-up step a real driver
/// performs, recorded by the register-backed io_mux descriptor.
const IO_MUX_GPIO37: *mut u32 = 0x500E_1000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        // 1. Route the pad (recorded; not electrically enforced at L1).
        core::ptr::write_volatile(IO_MUX_GPIO37, 1);

        // 2. Divisor for the console clock (explicit; matches reset).
        core::ptr::write_volatile(UART0_CLKDIV, 694);

        // 3. The observable: OK\n on UART0.
        for &b in b"OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(b));
        }
    }

    loop {}
}
