#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! GPIO8 header-pin smoke for the ESP32-P4X-Function-EV-Board.
//!
//! The image is loaded directly at its RISC-V entry point by the LabWired CLI;
//! it does not exercise the mask ROM or the SPI-flash boot path.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_BASE: usize = 0x500E_0000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO_FUNC8_OUT_SEL_CFG: *mut u32 = (GPIO_BASE + 0x578) as *mut u32;
const IO_MUX_GPIO8: *mut u32 = 0x500E_1024 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;

macro_rules! gpio8_pulse {
    () => {{
        unsafe {
            core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO8_MASK);
            core::arch::asm!("nop", options(nomem, nostack));
            core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
            core::arch::asm!("nop", options(nomem, nostack));
        }
    }};
}

#[entry]
fn main() -> ! {
    unsafe {
        // GPIO8 function select is 1. Keep drive strength at its default 2.
        core::ptr::write_volatile(IO_MUX_GPIO8, (2 << 10) | (1 << 12));
        // Select the GPIO_OUT latch and its output-enable path for GPIO8.
        core::ptr::write_volatile(GPIO_FUNC8_OUT_SEL_CFG, 256 | (1 << 10));
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);
    }

    // Emit a bounded pulse train, then park. This leaves a compact, stable
    // edge trace in the committed smoke artifact instead of filling its ring.
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    gpio8_pulse!();
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
