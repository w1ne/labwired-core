#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C6 HP-core UART0 console smoke.
//!
//! Target: `riscv32imc-unknown-none-elf`. The C6 HP core is RV32IMAC on silicon;
//! this smoke needs no atomics (A) and no multiply, so it links for the
//! `imc` subset the engine's interpreter is exercised against. The A extension
//! decodes (see `crates/core/src/decoder/riscv.rs`) but is deliberately not
//! part of this L1 image — the LP core and SMP are out of scope.
//!
//! Bring-up order mirrors what a real console path must do on the C6: PCR
//! gates the UART0 APB + function clock (there is no C3-style SYSTEM/APB_CTRL),
//! IO_MUX routes the pad, UART CLKDIV paces the shift, then bytes go through
//! the FIFO. Every address below is the chip descriptor's fact, cited in
//! `configs/chips/esp32c6.yaml`.

use panic_halt as _;
use riscv_rt::entry;

/// UART0 FIFO, 32-bit write pushes the low byte onto the TX FIFO.
/// esp-idf v5.3 `soc/esp32c6/include/soc/uart_reg.h` (`UART_FIFO_REG`),
/// base `DR_REG_UART_BASE` = 0x6000_0000.
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

/// PCR `UART0_CONF` @ +0x00 (bit0 CLK_EN, bit1 RST_EN) and
/// `UART0_SCLK_CONF` @ +0x04 (bit22 SCLK_EN). PCR base
/// `DR_REG_PCR_BASE` = 0x6009_6000 (esp32c6.svd `PCR`).
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;

/// IO_MUX pad register for GPIO16 / U0TXD at +0x44 (`MCU_SEL` = FUNC 1).
/// IO_MUX base `DR_REG_IO_MUX_BASE` = 0x6009_0000; U0TXD = GPIO16 per
/// ESP32-C6-DevKitC-1 v1.2 user guide (USB-to-UART bridge on U0TXD/U0RXD).
const IO_MUX_GPIO16: *mut u32 = 0x6009_0044 as *mut u32;

/// UART clock divider. Reset value 0x2B6 = 694 (esp32c6.svd UART0.CLKDIV).
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        // 1. Ungate UART0 through PCR: APB clock enable, then a
        //    read-modify-write of the function-clock register so the reset
        //    divider (0x0070_0000) survives and SCLK_EN is explicit.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        let sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, sclk | (1 << 22));

        // 2. Route the pad: GPIO16 MCU_SEL = 1 (U0TXD).
        core::ptr::write_volatile(IO_MUX_GPIO16, 1);

        // 3. Divisor for the console clock (explicit; matches reset).
        core::ptr::write_volatile(UART0_CLKDIV, 694);

        // 4. The observable: OK\n on UART0.
        for &b in b"OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(b));
        }
    }

    #[allow(clippy::empty_loop)] // bare-metal firmware: park the core forever
    loop {}
}
