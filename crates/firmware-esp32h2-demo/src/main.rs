#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-H2-DevKitM-1-N4S GPIO8 onboard WS2812 smoke.
//!
//! This direct-entry RV32IMC image uses only the documented GPIO output
//! registers and bit-bangs one red GRB frame to the board's GPIO8 LED. It does
//! not use the H2 ROM, radio, UART, USB, RMT, or a vendor HAL.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6009_100C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1024 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;

// The device model classifies WS2812 bits at 500 ns. At 96 MHz that boundary
// is 48 cycles. Account for MMIO and branch overhead around each NOP block:
// zero-high remains below the boundary and one-high remains above it.
#[inline(always)]
fn short_high() {
    unsafe { core::arch::asm!(".rept 24\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn long_high() {
    unsafe { core::arch::asm!(".rept 58\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn short_low() {
    unsafe { core::arch::asm!(".rept 90\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn long_low() {
    unsafe { core::arch::asm!(".rept 56\n nop\n .endr", options(nomem, nostack)) }
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
    // 8,000 NOPs take over 80 us at 96 MHz; the LED model latches after 40 us.
    for _ in 0..8_000 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);

        // WS2812 wire order is GRB: 0x00, 0xFF, 0x00 paints one red pixel.
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();
    }

    loop {}
}
