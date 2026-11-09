#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C6 DevKitC-1 UART0, built-in WS2812 RGB LED, and BOOT button smoke.
//!
//! Target: `riscv32imc-unknown-none-elf`. The C6 HP core is RV32IMAC on silicon;
//! this smoke needs no atomics (A) and no multiply, so it links for the
//! `imc` subset the engine's interpreter is exercised against. The A extension
//! decodes (see `crates/core/src/decoder/riscv.rs`) but is deliberately not
//! part of this L1 image — the LP core and SMP are out of scope.
//!
//! The smoke exercises UART0, sends a WS2812 frame through RMT channel 0 and
//! GPIO8's output matrix to the on-board RGB LED, then observes the active-low
//! BOOT button on GPIO9.

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
/// PCR `RMT_CONF` @ +0x2C and `RMT_SCLK_CONF` @ +0x30.
const PCR_RMT_CONF: *mut u32 = 0x6009_602C as *mut u32;
const PCR_RMT_SCLK_CONF: *mut u32 = 0x6009_6030 as *mut u32;

/// IO_MUX pad register for GPIO16 / U0TXD at +0x44 (`MCU_SEL` = FUNC 1).
/// IO_MUX base `DR_REG_IO_MUX_BASE` = 0x6009_0000; U0TXD = GPIO16 per
/// ESP32-C6-DevKitC-1 v1.2 user guide (USB-to-UART bridge on U0TXD/U0RXD).
const IO_MUX_GPIO16: *mut u32 = 0x6009_0044 as *mut u32;

/// UART clock divider. Reset value 0x2B6 = 694 (esp32c6.svd UART0.CLKDIV).
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

/// ESP32-C6 GPIO output-latch aliases and output enable.
const GPIO_OUT_W1TC: *mut u32 = 0x6009_100C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1024 as *mut u32;
const GPIO_ENABLE_W1TC: *mut u32 = 0x6009_1028 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;
const GPIO9_MASK: u32 = 1 << 9;
/// GPIO input register, one sampled pad level per bit.
const GPIO_IN: *const u32 = 0x6009_103C as *const u32;

/// GPIO8 output selector to RMT channel 0 (`RMT_SIG_OUT0_IDX=71` in ESP-IDF
/// `components/soc/esp32c6/include/soc/gpio_sig_map.h`).
const GPIO_FUNC8_OUT_SEL_CFG: *mut u32 = 0x6009_1574 as *mut u32;
const RMT_CH0_TX_CONF0: *mut u32 = 0x6000_6010 as *mut u32;
const RMT_CH0_TX_END: *const u32 = 0x6000_6038 as *const u32;
const RMT_CH0_INT_CLR: *mut u32 = 0x6000_6044 as *mut u32;
const RMT_MEM: *mut u32 = 0x6000_6400 as *mut u32;

/// IO_MUX GPIO8 register: base 0x6009_0000 + 0x04 + GPIO8 * 4.
const IO_MUX_GPIO8: *mut u32 = 0x6009_0024 as *mut u32;
/// IO_MUX pad register for GPIO9 (BOOT button): FUN_WPU and FUN_IE are bits 8/9.
const IO_MUX_GPIO9: *mut u32 = 0x6009_0028 as *mut u32;

#[inline(always)]
unsafe fn uart_write(bytes: &[u8]) {
    for &byte in bytes {
        core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
    }
}

#[inline(always)]
unsafe fn write_ws2812_byte(byte: u8, index: &mut usize) {
    for bit in (0..8).rev() {
        let one = byte & (1 << bit) != 0;
        // C6 RMT symbol: duration0[14:0], level0[15],
        // duration1[30:16], level1[31]. At the modeled 80 MHz RMT source,
        // each tick is two 160 MHz CPU cycles: a 20/50 tick high pulse is
        // 250/625 ns, and each full bit is 1.25 us.
        let high = if one { 50u32 } else { 20u32 };
        let low = if one { 50u32 } else { 80u32 };
        let symbol = high | (1 << 15) | (low << 16);
        core::ptr::write_volatile(RMT_MEM.add(*index), symbol);
        *index += 1;
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

        // Configure GPIO9 as a pulled-up input. The DevKitC-1 BOOT switch
        // shorts GPIO9 to ground while pressed; board_io supplies the contact
        // level to the GPIO model. IO_MUX currently records these pad bits but
        // does not electrically enforce them.
        let gpio9_mux = core::ptr::read_volatile(IO_MUX_GPIO9);
        core::ptr::write_volatile(IO_MUX_GPIO9, gpio9_mux | (1 << 8) | (1 << 9));
        core::ptr::write_volatile(GPIO_ENABLE_W1TC, GPIO9_MASK);

        // 4. Enable RMT's APB and function clocks. Preserve the PCR clock
        //    divider and select the default 80 MHz source.
        let rmt_conf = core::ptr::read_volatile(PCR_RMT_CONF);
        core::ptr::write_volatile(PCR_RMT_CONF, rmt_conf | 1);
        let rmt_sclk = core::ptr::read_volatile(PCR_RMT_SCLK_CONF);
        core::ptr::write_volatile(PCR_RMT_SCLK_CONF, rmt_sclk | (1 << 22));

        // GPIO8 is the DevKitC-1's on-board WS2812B data pin. Route it to RMT
        // channel 0, enable the output driver, and write GRB for solid red.
        core::ptr::write_volatile(IO_MUX_GPIO8, 0x0000_0A00);
        core::ptr::write_volatile(GPIO_FUNC8_OUT_SEL_CFG, 71);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);

        // 24 symbols, one per bit, followed by a zero-duration END marker.
        let mut symbol_index = 0;
        write_ws2812_byte(0x00, &mut symbol_index);
        write_ws2812_byte(0xFF, &mut symbol_index);
        write_ws2812_byte(0x00, &mut symbol_index);
        core::ptr::write_volatile(RMT_MEM.add(symbol_index), 0);

        // Divider 1, one memory block, idle output enabled/low. Program the
        // configuration before issuing the TX_START write trigger.
        let tx_conf = (1 << 8) | (1 << 16) | (1 << 6);
        core::ptr::write_volatile(RMT_CH0_TX_CONF0, tx_conf);
        core::ptr::write_volatile(RMT_CH0_TX_CONF0, tx_conf | 1);
        while core::ptr::read_volatile(RMT_CH0_TX_END) & 1 == 0 {}
        core::ptr::write_volatile(RMT_CH0_INT_CLR, 1);

        // Hold GPIO8 low past the WS2812 reset/latch interval.
        ws2812_reset_gap();

        // 5. UART0 capture confirms firmware reached the end of both outputs.
        uart_write(b"OK\n");
    }

    let (mut saw_press, mut saw_release) = (false, false);
    loop {
        let released = unsafe { core::ptr::read_volatile(GPIO_IN) & GPIO9_MASK != 0 };
        if !saw_press && !released {
            unsafe { uart_write(b"BOOT_BUTTON_PRESSED\n") };
            saw_press = true;
        } else if saw_press && !saw_release && released {
            unsafe { uart_write(b"BOOT_BUTTON_RELEASED\n") };
            saw_release = true;
        }
    }
}
