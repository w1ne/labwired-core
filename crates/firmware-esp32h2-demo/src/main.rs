#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Bare-ELF UART0 and onboard RGB smoke for the ESP32-H2-DevKitM-1-N4S.
//!
//! The ELF is loaded into the H2 flash window by LabWired's fast-boot runner.
//! This deliberately does not claim the mask-ROM/bootloader path or physical
//! USB-UART bridge. It exercises the UART0 FIFO and host-console capture, and
//! bit-bangs a red WS2812 frame through the modeled GPIO8 output latch.

use core::arch::asm;
use panic_halt as _;
use riscv_rt::entry;

const GPIO_BASE: usize = 0x6009_1000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;

// H2 PCR UART0_CONF @ 0x6009_6000 (CLK_EN bit 0), UART0_SCLK_CONF @ +0x04
// (SCLK_EN bit 22). Addresses and reset behavior are documented in
// configs/chips/esp32h2.yaml and the Espressif H2 register SVD.
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;

// H2 UART0 FIFO @ 0x6000_0000 and CLKDIV @ +0x14. UART0's register head
// matches the C6 through RXD_CNT; the H2 descriptor maps only that common
// region, before the differing CLK_CONF reset value at +0x88.
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
        // Keep the UART's APB/function clocks enabled, matching the H2 PCR
        // power-on state, and use the UART0 reset divisor for 115200 baud.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        let sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, sclk | (1 << 22));
        core::ptr::write_volatile(UART0_CLKDIV, 694);

        write_onboard_rgb_red();
        for &byte in b"H2_N4S_RGB_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    loop {}
}

unsafe fn write_onboard_rgb_red() {
    // Espressif's board guide wires the addressable RGB LED to GPIO8. The H2
    // profile models GPIO's OUT/ENABLE register head; IO_MUX and pad-level
    // electrical behavior are intentionally outside this smoke.
    core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
    core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);

    // WS2812 bytes are sent GRB, MSB first. One red pixel is 00 FF 00.
    for byte in [0x00u8, 0xFF, 0x00] {
        for bit in (0..8).rev() {
            pulse_ws2812_bit(byte & (1 << bit) != 0);
        }
    }

    // Hold low long enough for the device's reset/latch interval.
    nops!(4000);
}

#[inline(always)]
unsafe fn pulse_ws2812_bit(one: bool) {
    core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO8_MASK);
    if one {
        // Stay above the WS2812 observer's 500 ns data threshold at 96 MHz.
        nops!(55);
    } else {
        nops!(13);
    }
    core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
    if one {
        nops!(23);
    } else {
        nops!(42);
    }
}
