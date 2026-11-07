#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C61-DevKitC-1 v2.0 UART0 and GPIO8 RGB LED smoke.
//!
//! This bare-metal image runs on the HP RISC-V core and emits a marker through
//! UART0, which the board routes to its USB-to-UART bridge, and sends one red
//! WS2812B frame to the onboard RGB LED on GPIO8. It bypasses the ESP-ROM and
//! second-stage bootloader.

use panic_halt as _;
use riscv_rt::entry;

/// UART0 FIFO at DR_REG_UART0_BASE, from ESP-IDF C61 `uart_reg.h`.
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
/// UART0 PCR registers at DR_REG_PCR_BASE + 0x00/+0x04.
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
/// UART0_CLKDIV_SYNC at UART0 base + 0x14. PLL_F80M / 694 is approximately
/// 115,200 baud for 8N1.
const UART0_CLKDIV_SYNC: *mut u32 = 0x6000_0014 as *mut u32;
/// GPIO8 output path from ESP-IDF C61 `gpio_reg.h`.
const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6009_100C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1038 as *mut u32;
const GPIO_FUNC8_OUT_SEL_CFG: *mut u32 = 0x6009_1AF4 as *mut u32;
/// IO_MUX GPIO8 register and function selector from ESP-IDF C61 headers.
const IO_MUX_GPIO8: *mut u32 = 0x6009_0020 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;

// At 160 MHz these keep the WS2812B high phase below/above the model's 0/1
// threshold while keeping the complete bit cell near 1.25 us.
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
        // Enable UART0 and select C61's PLL_F80M function-clock source.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, (2 << 20) | (1 << 22));
        core::ptr::write_volatile(UART0_CLKDIV_SYNC, 694);
        for &byte in b"ESP32_C61_UART0_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }

        // GPIO8's IO_MUX Function 1 is GPIO8. GPIO_FUNC8_OUT_SEL=256 selects
        // the GPIO_OUT latch and OE_SEL=1 selects GPIO_ENABLE_REG[8].
        let mux = core::ptr::read_volatile(IO_MUX_GPIO8);
        core::ptr::write_volatile(IO_MUX_GPIO8, (mux & !(0b111 << 12)) | (1 << 12));
        core::ptr::write_volatile(GPIO_FUNC8_OUT_SEL_CFG, 256 | (1 << 10));
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);

        // The board's single WS2812B pixel expects GRB order; 00 FF 00 is red.
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();
    }

    loop {}
}
