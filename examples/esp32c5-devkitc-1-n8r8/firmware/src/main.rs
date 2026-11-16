#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Bare-ELF UART0 smoke for ESP32-C5-DevKitC-1-N8R8.
//!
//! This tests the C5 UART0 FIFO and LabWired host-console capture. It does not
//! test the mask-ROM boot path, bootloader, IO_MUX route, or physical bridge.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        for &byte in b"C5_N8R8_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
