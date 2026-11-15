#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Bare-ELF UART0 FIFO smoke for ESP32-P4X-C5-Function-EV-Board.
//!
//! This tests the P4 HP core's UART0 FIFO and simulator console capture. It
//! does not test mask-ROM startup, the bootloader, USB Serial/JTAG, UART pad
//! routing, or the board's C5 radio module.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x500C_A000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        for &byte in b"P4X_C5_FUNCTION_EV_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
