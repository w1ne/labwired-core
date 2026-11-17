#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Bare-ELF UART0 smoke for ESP8684-DevKitM-1-H4X / ESP32-C2.
//!
//! This exercises UART0's FIFO/status model. It does not test mask-ROM boot,
//! UART pad routing, Wi-Fi, BLE, or the board's RGB LED.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        for &byte in b"ESP8684_H4X_UART0_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
