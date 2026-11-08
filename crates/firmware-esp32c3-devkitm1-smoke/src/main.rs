#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Drive the ESP32-C3-DevKitM-1-N4X's on-board addressable RGB LED red.
//!
//! This smoke uses only the C3 GPIO output aliases and the modeled IO_MUX
//! register window with a 24-bit GRB single-wire frame. RMT is intentionally
//! not used or claimed.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_OUT_W1TS: *mut u32 = 0x6000_4008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6000_400C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6000_4024 as *mut u32;
const IO_MUX_GPIO8: *mut u32 = 0x6000_9024 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;

// At 160 MHz the model treats high pulses below 500 ns as a 0 and longer
// pulses as a 1. Keep the WS2812 bit cell near 1.25 us.
#[inline(always)]
fn short_high() {
    unsafe { core::arch::asm!(".rept 40\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn long_high() {
    unsafe { core::arch::asm!(".rept 96\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn short_low() {
    unsafe { core::arch::asm!(".rept 150\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn long_low() {
    unsafe { core::arch::asm!(".rept 94\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
unsafe fn send_ws2812_byte(byte: u8) {
    for bit in (0..8).rev() {
        let one = (byte & (1 << bit)) != 0;
        core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO8_MASK);
        if one {
            long_high();
        } else {
            short_high();
        }
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
        if one {
            long_low();
        } else {
            short_low();
        }
    }
}

#[inline(never)]
fn ws2812_reset_gap() {
    for _ in 0..8_000 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // GPIO8's IO_MUX reset value selects its GPIO function. Writing it
        // explicitly keeps the board smoke's pin setup visible in the ELF.
        let gpio8_mux = core::ptr::read_volatile(IO_MUX_GPIO8);
        core::ptr::write_volatile(IO_MUX_GPIO8, gpio8_mux & !(0b111 << 12));
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);

        // The board LED accepts a 24-bit GRB frame. 00 FF 00 is red.
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();
    }

    loop {}
}
