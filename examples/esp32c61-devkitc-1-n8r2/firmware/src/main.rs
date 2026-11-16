#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Bare-ELF UART0 smoke for ESP32-C61-DevKitC-1-N8R2.
//!
//! This exercises the UART0 FIFO and LabWired's host-console capture. It does
//! not exercise mask-ROM boot, IO_MUX pad routing, or the physical USB bridge.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        for &byte in b"C61_N8R2_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
