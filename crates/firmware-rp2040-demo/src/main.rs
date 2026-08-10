#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2040 Peripheral Base Addresses
const UART0_BASE: u32 = 0x40034000;
const RESETS_BASE: u32 = 0x4000C000;
const CLOCKS_BASE: u32 = 0x40008000;
const IO_BANK0_BASE: u32 = 0x40014000;
// We'll use a mocked "LED" mapped to a scratch register or standard PIO base for demonstration
const MOCK_LED_REG: *mut u32 = 0x50200000 as *mut u32;

// RESETS (RP2040 datasheet §2.14, pp.174-178): RESET @ 0x00, RESET_DONE @ 0x08.
// Every peripheral comes out of power-on HELD IN RESET — RESET's reset value is
// all-ones — and software must deassert what it intends to use.
const RESETS_RESET: *mut u32 = RESETS_BASE as *mut u32;
const RESETS_RESET_DONE: *const u32 = (RESETS_BASE + 0x08) as *const u32;
const RESETS_IO_BANK0: u32 = 1 << 5;
const RESETS_PADS_BANK0: u32 = 1 << 8;
const RESETS_UART0: u32 = 1 << 22;

// CLOCKS.CLK_PERI_CTRL @ 0x48 (datasheet Table 226, p.204). ENABLE is bit 11
// and resets to 0, so clk_peri — the UART's UARTCLK — is STOPPED at boot.
// AUXSRC[7:5] resets to 0x0 = CLK_SYS, which is what we want, so only ENABLE
// has to be written.
const CLK_PERI_CTRL: *mut u32 = (CLOCKS_BASE + 0x48) as *mut u32;
const CLK_PERI_CTRL_ENABLE: u32 = 1 << 11;

// IO_BANK0 GPIOn_CTRL @ 0x04 + 8*n (datasheet Table 283, p.245). FUNCSEL is
// bits [4:0] and resets to 0x1F = NULL: the pad is connected to nothing at all
// until firmware selects a function.
const GPIO0_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04) as *mut u32;
/// GP0 function F2 is `UART0 TX` (datasheet Table 279, p.238). F1 is SPI0 RX
/// and F5 is SIO, so the number is not interchangeable with another pin's.
const GPIO_FUNC_UART: u32 = 2;

// UART0 is a PL011 (datasheet Table 425, p.430).
const UART0_DR: *mut u32 = UART0_BASE as *mut u32;
const UART0_IBRD: *mut u32 = (UART0_BASE + 0x24) as *mut u32;
const UART0_FBRD: *mut u32 = (UART0_BASE + 0x28) as *mut u32;
const UART0_LCR_H: *mut u32 = (UART0_BASE + 0x2C) as *mut u32;
const UART0_CR: *mut u32 = (UART0_BASE + 0x30) as *mut u32;

/// `UARTLCR_H`: WLEN = 0b11 (8 data bits, bits [6:5]) + FEN (bit 4). No parity,
/// one stop bit — 8N1 (datasheet Table 432, p.433).
const LCR_H_8N1_FIFO: u32 = (0b11 << 5) | (1 << 4);
/// `UARTCR`: UARTEN (bit 0) + TXE (bit 8). UARTEN **resets to 0** (datasheet
/// Table 433, p.435), so without this write the transmitter never runs at all.
const CR_UARTEN_TXE: u32 = (1 << 0) | (1 << 8);

/// Baud divisor for 115 200 from clk_peri.
///
/// This firmware brings up neither XOSC nor a PLL, so clk_sys — and therefore
/// clk_peri, which we source from it — is the ring oscillator the chip boots
/// on: "clk_sys and clk_ref are now running at a relatively low frequency
/// (typically 6.5MHz)" (RP2040 datasheet §2.13.2, p.129; §2.17.2, p.223 gives
/// the same nominal). That is above the 3.6864 MHz floor the fractional
/// divider needs for standard baud rates (§4.2.3.2.1, p.422).
///
/// Baud Rate Divisor = UARTCLK / (16 × baud) = 6_500_000 / (16 × 115_200)
/// = 3.5264. The integer part goes in `UARTIBRD`; the fraction is
/// round(0.5264 × 64) = 34 in `UARTFBRD` — the datasheet's own worked example
/// on p.429 uses exactly this arithmetic for a 125 MHz clk_peri.
///
/// Realised baud = 6_500_000 / (16 × (3 + 34/64)) = 115 044, 0.14% slow.
///
/// ⚠️ The ROSC is untrimmed and varies with process, voltage and temperature
/// (§2.15.2, p.181: "typically 6MHz but varies with PVT"), so this divisor is
/// only as accurate as that oscillator. Firmware that must hold baud across
/// parts has to switch clk_sys to the 12 MHz XOSC and a PLL first, which is a
/// clock-tree bring-up this smoke demo deliberately does not carry.
const UART_IBRD_115200: u32 = 3;
const UART_FBRD_115200: u32 = 34;

/// Bring UART0 up the way silicon requires: out of reset, clocked, muxed onto a
/// pad, given a divisor, and only then enabled.
///
/// The whole of this file's UART surface used to be `write_volatile(UART0_DR,
/// b)`. That prints on LabWired's permissive UART model and transmits nothing
/// whatsoever on a real RP2040: UART0 is held in reset, clk_peri is stopped,
/// GP0's FUNCSEL is NULL, both baud divisors read 0, and `UARTCR.UARTEN` is 0.
/// It is also why a logic-analyzer probe on GP0 read a flat line while the
/// transaction-level bus monitor decoded the traffic fine — with no live pad
/// route the probe sees the pad's own latch, and with a zero divisor there is
/// no bit period to narrate.
fn uart0_init() {
    unsafe {
        // 1. Deassert reset for UART0 and the IO/pad banks that carry GP0, then
        //    wait for the controller to acknowledge (RESET_DONE is the inverse
        //    of RESET, datasheet Table 204, p.178).
        let bits = RESETS_UART0 | RESETS_IO_BANK0 | RESETS_PADS_BANK0;
        let reset = core::ptr::read_volatile(RESETS_RESET);
        core::ptr::write_volatile(RESETS_RESET, reset & !bits);
        while core::ptr::read_volatile(RESETS_RESET_DONE) & bits != bits {}

        // 2. Start clk_peri (AUXSRC already selects clk_sys out of reset).
        let peri = core::ptr::read_volatile(CLK_PERI_CTRL);
        core::ptr::write_volatile(CLK_PERI_CTRL, peri | CLK_PERI_CTRL_ENABLE);

        // 3. Mux GP0 to UART0 TX. Until this write the pad's FUNCSEL is NULL.
        core::ptr::write_volatile(GPIO0_CTRL, GPIO_FUNC_UART);

        // 4. Program the divisor, then the frame format. A write to UARTLCR_H
        //    is what latches IBRD/FBRD into the baud generator, so it must come
        //    after them.
        core::ptr::write_volatile(UART0_IBRD, UART_IBRD_115200);
        core::ptr::write_volatile(UART0_FBRD, UART_FBRD_115200);
        core::ptr::write_volatile(UART0_LCR_H, LCR_H_8N1_FIFO);

        // 5. Enable the UART and its transmitter.
        core::ptr::write_volatile(UART0_CR, CR_UARTEN_TXE);
    }
}

#[entry]
fn main() -> ! {
    let mut led_state = 0u32;

    uart0_init();

    loop {
        // "Blink" the mocked LED by toggling the register
        led_state ^= 1;
        unsafe {
            core::ptr::write_volatile(MOCK_LED_REG, led_state);
        }

        // Write a message to UART
        print_uart("RP2040_SMOKE_OK\n");

        // Small delay
        for _ in 0..1000u32 {
            cortex_m::asm::nop();
        }
    }
}

fn print_uart(s: &str) {
    for b in s.bytes() {
        unsafe {
            core::ptr::write_volatile(UART0_DR, b as u32);
        }
    }
}
