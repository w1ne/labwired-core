#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Bare-metal onboard-LED smoke for the Seeed Studio XIAO ESP32-C6.

use panic_halt as _;
use riscv_rt::entry;

const IO_MUX_GPIO15: *mut u32 = 0x6009_0040 as *mut u32;
const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6009_100C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1024 as *mut u32;
const GPIO_FUNC15_OUT_SEL_CFG: *mut u32 = 0x6009_1590 as *mut u32;
const LED: u32 = 1 << 15;
const SIG_GPIO_OUT: u32 = 128;

#[entry]
fn main() -> ! {
    unsafe {
        // Select the GPIO function and plain GPIO output path for GPIO15.
        core::ptr::write_volatile(IO_MUX_GPIO15, 1);
        core::ptr::write_volatile(GPIO_FUNC15_OUT_SEL_CFG, SIG_GPIO_OUT);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, LED);

        loop {
            // The XIAO's yellow user LED is wired from 3V3 through a resistor
            // to GPIO15, so driving the pin low lights it.
            core::ptr::write_volatile(GPIO_OUT_W1TC, LED);
            delay();
            core::ptr::write_volatile(GPIO_OUT_W1TS, LED);
            delay();
        }
    }
}

#[inline(never)]
fn delay() {
    for _ in 0..2_000 {
        // Keep a bounded instruction loop in the captured firmware trace.
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
