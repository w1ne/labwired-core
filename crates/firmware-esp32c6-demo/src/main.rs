#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C6 DevKitC-1 UART0 + built-in WS2812 RGB LED smoke.
//!
//! Target: `riscv32imc-unknown-none-elf`. The C6 HP core is RV32IMAC on silicon;
//! this smoke needs no atomics (A) and no multiply, so it links for the
//! `imc` subset the engine's interpreter is exercised against. The A extension
//! decodes (see `crates/core/src/decoder/riscv.rs`) but is deliberately not
//! part of this L1 image — the LP core and SMP are out of scope.
//!
//! The smoke exercises UART0 and bit-bangs a WS2812 frame on the DevKitC-1's
//! on-board GPIO8 LED. RMT is not modeled on the C6, so the RGB path uses the
//! real GPIO set/clear registers and a cycle-counted waveform instead.

use panic_halt as _;
use riscv_rt::entry;

/// UART0 FIFO, 32-bit write pushes the low byte onto the TX FIFO.
/// esp-idf v5.3 `soc/esp32c6/include/soc/uart_reg.h` (`UART_FIFO_REG`),
/// base `DR_REG_UART_BASE` = 0x6000_0000.
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

/// PCR `UART0_CONF` @ +0x00 (bit0 CLK_EN, bit1 RST_EN) and
/// `UART0_SCLK_CONF` @ +0x04 (bit22 SCLK_EN). PCR base
/// `DR_REG_PCR_BASE` = 0x6009_6000 (esp32c6.svd `PCR`).
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;

/// IO_MUX pad register for GPIO16 / U0TXD at +0x44 (`MCU_SEL` = FUNC 1).
/// IO_MUX base `DR_REG_IO_MUX_BASE` = 0x6009_0000; U0TXD = GPIO16 per
/// ESP32-C6-DevKitC-1 v1.2 user guide (USB-to-UART bridge on U0TXD/U0RXD).
const IO_MUX_GPIO16: *mut u32 = 0x6009_0044 as *mut u32;

/// UART clock divider. Reset value 0x2B6 = 694 (esp32c6.svd UART0.CLKDIV).
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

/// ESP32-C6 GPIO output-latch aliases and output enable.
const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6009_100C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1024 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;

/// IO_MUX GPIO8 register: base 0x6009_0000 + 0x04 + GPIO8 * 4.
const IO_MUX_GPIO8: *mut u32 = 0x6009_0024 as *mut u32;

// At 160 MHz, the WS2812 model classifies a bit by whether its HIGH pulse is
// shorter or longer than 500 ns (80 core cycles). The surrounding MMIO writes
// and branches contribute a few cycles to each pulse. These static NOP blocks
// leave zero below and one above that threshold while keeping each bit close
// to the WS2812's 1.25 us cell time.
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
    // The generic WS2812 model requires a low reset/latch gap of 40 us. This
    // loop takes over 100 us at the modeled 160 MHz even with loop control.
    for _ in 0..8_000 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // 1. Ungate UART0 through PCR: APB clock enable, then a
        //    read-modify-write of the function-clock register so the reset
        //    divider (0x0070_0000) survives and SCLK_EN is explicit.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        let sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, sclk | (1 << 22));

        // 2. Route the pad: GPIO16 MCU_SEL = 1 (U0TXD).
        core::ptr::write_volatile(IO_MUX_GPIO16, 1);

        // 3. Divisor for the console clock (explicit; matches reset).
        core::ptr::write_volatile(UART0_CLKDIV, 694);

        // 4. GPIO8 is the DevKitC-1's on-board WS2812B data pin. Send GRB for
        //    solid red (G=0, R=255, B=0), then hold low through the reset gap.
        core::ptr::write_volatile(IO_MUX_GPIO8, 0x0000_0A00);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();

        // 5. UART0 capture confirms firmware reached the end of both outputs.
        for &b in b"OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(b));
        }
    }

    loop {}
}
