#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::arch::asm;
use panic_halt as _;
use riscv_rt::entry;

const GPIO_BASE: usize = 0x6000_4000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

macro_rules! nops {
    ($count:literal) => {
        unsafe {
            asm!(
                concat!(".rept ", stringify!($count), "\n nop\n .endr"),
                options(nomem, nostack)
            )
        }
    };
}

#[entry]
fn main() -> ! {
    unsafe {
        // 80 MHz APB / 115200 baud. Configure the reset console divisor for
        // this fast-boot fixture before sending its marker.
        core::ptr::write_volatile(UART0_CLKDIV, 694);
        write_onboard_rgb_red();
        for &byte in b"C3_N4X_RGB_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {}
}

unsafe fn write_onboard_rgb_red() {
    // Espressif's DevKitM-1-N4X guide maps its addressable LED to GPIO8. The
    // system profile observes GPIO edges with a WS2812 model; this does not
    // claim IO_MUX, electrical, or RMT behavior.
    core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
    core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);

    // WS2812 sends GRB, MSB first. A second black frame begins after the reset
    // gap so the observer latches the red frame before the smoke checks it.
    write_ws2812_frame([0x00, 0xFF, 0x00]);
    nops!(8000); // > 40 us at the C3's modeled 160 MHz clock
    write_ws2812_frame([0x00, 0x00, 0x00]);
    nops!(8000);
}

unsafe fn write_ws2812_frame(bytes: [u8; 3]) {
    for byte in bytes {
        for bit in (0..8).rev() {
            pulse_ws2812_bit(byte & (1 << bit) != 0);
        }
    }
}

#[inline(always)]
unsafe fn pulse_ws2812_bit(one: bool) {
    core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO8_MASK);
    if one {
        // At 160 MHz, 100 nops plus the store/branch exceed the model's 500 ns
        // threshold. A zero pulse stays below it with 35 nops.
        nops!(100);
    } else {
        nops!(35);
    }
    core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
    if one {
        nops!(45);
    } else {
        nops!(110);
    }
}
