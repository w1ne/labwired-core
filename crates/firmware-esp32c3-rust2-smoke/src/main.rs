#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Drive the ESP32-C3-DevKit-RUST-2's on-board WS2812B RGB LED red on GPIO2.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_OUT_W1TS: *mut u32 = 0x6000_4008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6000_400C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6000_4024 as *mut u32;
const IO_MUX_GPIO2: *mut u32 = 0x6000_900C as *mut u32;
const GPIO2_MASK: u32 = 1 << 2;

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
        core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO2_MASK);
        if one {
            long_high();
        } else {
            short_high();
        }
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO2_MASK);
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
        // GPIO2's IO_MUX reset value selects its GPIO function; make that
        // selection explicit before enabling its output driver.
        let gpio2_mux = core::ptr::read_volatile(IO_MUX_GPIO2);
        core::ptr::write_volatile(IO_MUX_GPIO2, gpio2_mux & !(0b111 << 12));
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO2_MASK);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO2_MASK);

        // The board's RGB LED is a WS2812B. 00 FF 00 is red in GRB order.
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();
    }

    loop {}
}
