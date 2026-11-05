#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Exercise the three active-low discrete RGB LED channels on the
//! ESP8684-DevKitM-1-H4X through the modeled GPIO and IO_MUX registers.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_BASE: usize = 0x6000_4000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO_FUNC_OUT_SEL_BASE: usize = GPIO_BASE + 0x554;
const IO_MUX_BASE: usize = 0x6000_9000;
const SIG_GPIO_OUT: u32 = 128;
const GPIO_MASKS: [u32; 3] = [1 << 0, 1 << 1, 1 << 8];
const IO_MUX_OFFSETS: [usize; 3] = [0x04, 0x08, 0x24];

#[inline(never)]
fn delay() {
    for _ in 0..300 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) }
    }
}

#[inline(never)]
unsafe fn pulse(mask: u32) {
    // The board's common-anode RGB LED turns a channel on when its GPIO sinks
    // current (low), and off when the GPIO drives high.
    core::ptr::write_volatile(GPIO_OUT_W1TC, mask);
    delay();
    core::ptr::write_volatile(GPIO_OUT_W1TS, mask);
    delay();
}

#[entry]
fn main() -> ! {
    unsafe {
        for (pin, mux_offset) in [0usize, 1, 8].into_iter().zip(IO_MUX_OFFSETS) {
            let mux = (IO_MUX_BASE + mux_offset) as *mut u32;
            let value = core::ptr::read_volatile(mux);
            core::ptr::write_volatile(mux, value & !(0b111 << 12));

            let out_sel = (GPIO_FUNC_OUT_SEL_BASE + pin * 4) as *mut u32;
            core::ptr::write_volatile(out_sel, SIG_GPIO_OUT);
        }

        let all_channels = GPIO_MASKS[0] | GPIO_MASKS[1] | GPIO_MASKS[2];
        core::ptr::write_volatile(GPIO_OUT_W1TS, all_channels);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, all_channels);

        // Exercise red, green, and blue independently. GPIO8 is a strapping
        // pin, but this firmware changes it only after the chip has booted.
        for mask in GPIO_MASKS {
            for _ in 0..6 {
                pulse(mask);
            }
        }
    }

    loop {}
}
