#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Bare-ELF UART0 smoke for the ESP32-H2-DevKitM-1-N4S.
//!
//! The ELF is loaded into the H2 flash window by LabWired's fast-boot runner.
//! This deliberately does not claim the mask-ROM/bootloader path or physical
//! USB-UART bridge. It exercises the UART0 FIFO and host-console capture.

use panic_halt as _;
use riscv_rt::entry;

// H2 PCR UART0_CONF @ 0x6009_6000 (CLK_EN bit 0), UART0_SCLK_CONF @ +0x04
// (SCLK_EN bit 22). Addresses and reset behavior are documented in
// configs/chips/esp32h2.yaml and the Espressif H2 register SVD.
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;

// H2 UART0 FIFO @ 0x6000_0000 and CLKDIV @ +0x14. UART0's register head
// matches the C6 through RXD_CNT; the H2 descriptor maps only that common
// region, before the differing CLK_CONF reset value at +0x88.
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        // Keep the UART's APB/function clocks enabled, matching the H2 PCR
        // power-on state, and use the UART0 reset divisor for 115200 baud.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        let sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, sclk | (1 << 22));
        core::ptr::write_volatile(UART0_CLKDIV, 694);

        for &byte in b"H2_N4S_UART_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {}
}
