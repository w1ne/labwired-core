#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C5-DevKitC-1 v1.2 UART0 TX and GPIO27 RGB LED smoke.
//!
//! This bare-metal image runs on the HP RISC-V core, sends a UART0 marker, and
//! bit-bangs one red WS2812 frame to the board's GPIO27 RGB LED. It does not
//! start the ESP-IDF ROM/bootloader.

use panic_halt as _;
use riscv_rt::entry;

/// UART0 FIFO at DR_REG_UART0_BASE, from ESP-IDF v6.0 C5 `uart_reg.h`.
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
/// UART0 PCR registers at DR_REG_PCR_BASE + 0x00/+0x04.
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
/// UART0_CLKDIV_SYNC at UART0 base + 0x14. PLL_F80M / 694 gives approximately
/// 115,200 baud for 8N1.
const UART0_CLKDIV_SYNC: *mut u32 = 0x6000_0014 as *mut u32;
/// GPIO27 direct output path from ESP-IDF C5 `gpio_reg.h` and the C5 TRM.
const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6009_100C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1038 as *mut u32;
const GPIO_FUNC27_OUT_SEL_CFG: *mut u32 = 0x6009_1B30 as *mut u32;
/// ESP32-C5 IO_MUX GPIO27 register.
const IO_MUX_GPIO27: *mut u32 = 0x6009_006C as *mut u32;
const GPIO27_MASK: u32 = 1 << 27;

// At 240 MHz these keep the WS2812B high phase below/above the model's 500 ns
// zero/one threshold while keeping each bit cell near 1.25 us.
#[inline(always)]
fn short_high() {
    unsafe { core::arch::asm!(".rept 60\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn long_high() {
    unsafe { core::arch::asm!(".rept 144\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn short_low() {
    unsafe { core::arch::asm!(".rept 225\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn long_low() {
    unsafe { core::arch::asm!(".rept 141\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
unsafe fn send_ws2812_byte(byte: u8) {
    for bit in (0..8).rev() {
        let one = (byte & (1 << bit)) != 0;
        core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO27_MASK);
        if one {
            long_high();
        } else {
            short_high();
        }
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO27_MASK);
        if one {
            long_low();
        } else {
            short_low();
        }
    }
}

#[inline(never)]
fn ws2812_reset_gap() {
    for _ in 0..12_000 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable the UART function clock and select the fixed 80 MHz PLL
        // source so the smoke does not depend on a 40/48 MHz XTAL selection.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, (2 << 20) | (1 << 22));
        core::ptr::write_volatile(UART0_CLKDIV_SYNC, 694);
        for &byte in b"ESP32_C5_UART0_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }

        // GPIO27's IO_MUX Function 1 is GPIO27. GPIO_FUNC27_OUT_SEL selects
        // the GPIO_OUT latch and OE_SEL selects GPIO_ENABLE_REG[27].
        let mux = core::ptr::read_volatile(IO_MUX_GPIO27);
        core::ptr::write_volatile(IO_MUX_GPIO27, (mux & !(0b111 << 12)) | (1 << 12));
        core::ptr::write_volatile(GPIO_FUNC27_OUT_SEL_CFG, 256 | (1 << 10));
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO27_MASK);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO27_MASK);

        // The onboard pixel expects GRB wire order; 00 FF 00 is red.
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();
    }

    loop {}
}
