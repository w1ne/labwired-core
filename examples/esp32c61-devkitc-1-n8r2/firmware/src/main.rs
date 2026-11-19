#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Bare-ELF UART0 and GPIO4 smoke for ESP32-C61-DevKitC-1-N8R2.
//!
//! This exercises the C61 GPIO4 output latch/enable and UART0 FIFO. It does
//! not exercise mask-ROM boot, IO_MUX pad routing, the RGB LED protocol, or
//! the physical USB bridge.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1038 as *mut u32;
const GPIO4: u32 = 1 << 4;

#[entry]
fn main() -> ! {
    unsafe {
        // GPIO4 is the DevKitC-1 v2.0 J1 pin 3; the smoke checks its register
        // state and makes no claim about IO_MUX or electrical pin behavior.
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO4);
        core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO4);

        for &byte in b"C61_N8R2_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
