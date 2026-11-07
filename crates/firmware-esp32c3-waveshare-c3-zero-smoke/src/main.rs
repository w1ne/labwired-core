#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Drive the Waveshare ESP32-C3-Zero's onboard WS2812 RGB LED red on GPIO10.
//!
//! The firmware uses the C3 GPIO output aliases and IO_MUX register window to
//! bit-bang a 24-bit GRB frame. It does not use or claim the RMT controller.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_OUT_W1TS: *mut u32 = 0x6000_4008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6000_400C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6000_4024 as *mut u32;
const IO_MUX_GPIO10: *mut u32 = 0x6000_902C as *mut u32;
const GPIO10_MASK: u32 = 1 << 10;

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
        core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO10_MASK);
        if one {
            long_high();
        } else {
            short_high();
        }
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO10_MASK);
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
        // GPIO10's IO_MUX reset value selects GPIO. Clear the function field
        // explicitly before enabling output for the board's RGB data line.
        let gpio10_mux = core::ptr::read_volatile(IO_MUX_GPIO10);
        core::ptr::write_volatile(IO_MUX_GPIO10, gpio10_mux & !(0b111 << 12));
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO10_MASK);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO10_MASK);

        // The WS2812 accepts a 24-bit GRB frame; 00 FF 00 selects red.
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();
    }

    loop {}
}
