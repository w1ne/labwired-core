#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C61-DevKitC-1 v2.0 UART0 TX smoke.
//!
//! This bare-metal image runs on the HP RISC-V core and emits a marker through
//! UART0, which the board routes to its USB-to-UART bridge. It bypasses the
//! ESP-ROM and second-stage bootloader.

use panic_halt as _;
use riscv_rt::entry;

/// UART0 FIFO at DR_REG_UART0_BASE, from ESP-IDF C61 `uart_reg.h`.
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
/// UART0 PCR registers at DR_REG_PCR_BASE + 0x00/+0x04.
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
/// UART0_CLKDIV_SYNC at UART0 base + 0x14. PLL_F80M / 694 is approximately
/// 115,200 baud for 8N1.
const UART0_CLKDIV_SYNC: *mut u32 = 0x6000_0014 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable UART0 and select C61's PLL_F80M function-clock source.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, (2 << 20) | (1 << 22));
        core::ptr::write_volatile(UART0_CLKDIV_SYNC, 694);
        for &byte in b"ESP32_C61_UART0_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {}
}
