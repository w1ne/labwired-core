#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C6-DevKitM-1-N4 UART0 and onboard WS2812B RGB LED smoke.
//!
//! The image targets the HP core's RV32IMC instruction subset. It sends one
//! solid-red frame through RMT channel 0 and GPIO8, then writes a UART marker.
//! ROM boot, the LP core, radio, and USB paths are outside this smoke.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
const PCR_RMT_CONF: *mut u32 = 0x6009_602C as *mut u32;
const PCR_RMT_SCLK_CONF: *mut u32 = 0x6009_6030 as *mut u32;
const IO_MUX_GPIO16: *mut u32 = 0x6009_0044 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6009_100C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1024 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;
const GPIO_FUNC8_OUT_SEL_CFG: *mut u32 = 0x6009_1574 as *mut u32;
const RMT_CH0_TX_CONF0: *mut u32 = 0x6000_6010 as *mut u32;
const RMT_CH0_TX_END: *const u32 = 0x6000_6038 as *const u32;
const RMT_CH0_INT_CLR: *mut u32 = 0x6000_6044 as *mut u32;
const RMT_MEM: *mut u32 = 0x6000_6400 as *mut u32;
const IO_MUX_GPIO8: *mut u32 = 0x6009_0024 as *mut u32;

#[inline(always)]
unsafe fn write_ws2812_byte(byte: u8, index: &mut usize) {
    for bit in (0..8).rev() {
        let one = byte & (1 << bit) != 0;
        // At the modeled 80 MHz RMT source, ticks are 12.5 ns. These periods
        // encode a 250/625 ns high pulse and a 1.25 us WS2812 bit cell.
        let high = if one { 50u32 } else { 20u32 };
        let low = if one { 50u32 } else { 80u32 };
        let symbol = high | (1 << 15) | (low << 16);
        core::ptr::write_volatile(RMT_MEM.add(*index), symbol);
        *index += 1;
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
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        let uart_sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, uart_sclk | (1 << 22));
        core::ptr::write_volatile(IO_MUX_GPIO16, 1);
        core::ptr::write_volatile(UART0_CLKDIV, 694);

        let rmt_conf = core::ptr::read_volatile(PCR_RMT_CONF);
        core::ptr::write_volatile(PCR_RMT_CONF, rmt_conf | 1);
        let rmt_sclk = core::ptr::read_volatile(PCR_RMT_SCLK_CONF);
        core::ptr::write_volatile(PCR_RMT_SCLK_CONF, rmt_sclk | (1 << 22));

        // ESP32-C6-DevKitM-1 schematic: D6 WS2812B DIN is GPIO8.
        core::ptr::write_volatile(IO_MUX_GPIO8, 0x0000_0A00);
        core::ptr::write_volatile(GPIO_FUNC8_OUT_SEL_CFG, 71);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);

        let mut symbol_index = 0;
        write_ws2812_byte(0x00, &mut symbol_index); // G
        write_ws2812_byte(0xFF, &mut symbol_index); // R
        write_ws2812_byte(0x00, &mut symbol_index); // B
        core::ptr::write_volatile(RMT_MEM.add(symbol_index), 0);

        let tx_conf = (1 << 8) | (1 << 16) | (1 << 6);
        core::ptr::write_volatile(RMT_CH0_TX_CONF0, tx_conf);
        core::ptr::write_volatile(RMT_CH0_TX_CONF0, tx_conf | 1);
        while core::ptr::read_volatile(RMT_CH0_TX_END) & 1 == 0 {}
        core::ptr::write_volatile(RMT_CH0_INT_CLR, 1);
        ws2812_reset_gap();

        for &b in b"ESP32_C6_DEVKITM_N4_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(b));
        }
    }

    loop {}
}
