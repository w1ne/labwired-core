//! ILI9341 TFT Lab — STM32F103
//!
//! Demonstrates the ILI9341 240×320 RGB565 display simulator:
//!   1. SPI1 + PA4 (CS) initialisation
//!   2. ILI9341 init sequence (SLPOUT → COLMOD → DISPON)
//!   3. CASET / PASET window set + RAMWR pixel write
//!   4. Two 240×16 horizontal colour bands visible in the canvas widget:
//!        Row 0..15  — eight equal-width vertical colour bars (EBU test pattern)
//!        Row 16..31 — solid bright red (0xF800)
//!   5. Continuous loop printing "frame done" to UART
//!
//! Pin mapping (SPI1 on STM32F103):
//!   PA4  — CS      (GPIO output push-pull)
//!   PA5  — SCK     (AF push-pull)
//!   PA6  — MISO    (input floating — not used by ILI9341 in write-only mode)
//!   PA7  — MOSI    (AF push-pull)

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// ----- Register addresses -------------------------------------------------

const RCC_APB2ENR: *mut u32  = 0x4002_1018 as *mut u32;
const GPIOA_CRL:   *mut u32  = 0x4001_0800 as *mut u32;
const GPIOA_BSRR:  *mut u32  = 0x4001_0810 as *mut u32;
const GPIOA_BRR:   *mut u32  = 0x4001_0814 as *mut u32;
const SPI1_CR1:    *mut u16  = 0x4001_3000 as *mut u16;
const SPI1_SR:     *const u16 = 0x4001_3008 as *const u16;
const SPI1_DR:     *mut u16  = 0x4001_300C as *mut u16;
const UART1_DR:    *mut u8   = (0x4001_3800 + 0x04) as *mut u8;

// ----- UART helpers -------------------------------------------------------

fn uart_byte(b: u8) {
    unsafe { core::ptr::write_volatile(UART1_DR, b) }
}

fn uart_str(s: &str) {
    for b in s.bytes() {
        uart_byte(b);
    }
}

// ----- SPI helpers --------------------------------------------------------

/// Write one byte via SPI1 (MOSI only — MISO is discarded for display-write).
fn spi_write(byte: u8) {
    // Wait until TXE (bit 1)
    for _ in 0..2048 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        if sr & 0x0002 != 0 { break; }
    }
    unsafe { core::ptr::write_volatile(SPI1_DR, byte as u16) };
    // Wait until RXNE (bit 0) so we don't over-fill the TX FIFO
    for _ in 0..2048 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        if sr & 0x0001 != 0 { break; }
    }
    // Drain RX
    let _ = unsafe { core::ptr::read_volatile(SPI1_DR) };
}

fn cs_low()  { unsafe { core::ptr::write_volatile(GPIOA_BRR,  1 << 4) } }
fn cs_high() { unsafe { core::ptr::write_volatile(GPIOA_BSRR, 1 << 4) } }

// ----- ILI9341 protocol ---------------------------------------------------
//
// The simulator's D/C pin is implicit in the command state machine:
// the first byte after cs_low is always the command byte.

fn tft_cmd(cmd: u8) {
    cs_low();
    spi_write(cmd);
    cs_high();
}

fn tft_cmd1(cmd: u8, p0: u8) {
    cs_low();
    spi_write(cmd);
    spi_write(p0);
    cs_high();
}

fn tft_cmd4(cmd: u8, p0: u8, p1: u8, p2: u8, p3: u8) {
    cs_low();
    spi_write(cmd);
    spi_write(p0);
    spi_write(p1);
    spi_write(p2);
    spi_write(p3);
    cs_high();
}

/// Set the pixel-write addressing window.
/// col_start..=col_end, row_start..=row_end.
fn tft_set_window(col_start: u16, col_end: u16, row_start: u16, row_end: u16) {
    // CASET
    tft_cmd4(
        0x2A,
        (col_start >> 8) as u8,
        col_start as u8,
        (col_end >> 8) as u8,
        col_end as u8,
    );
    // PASET
    tft_cmd4(
        0x2B,
        (row_start >> 8) as u8,
        row_start as u8,
        (row_end >> 8) as u8,
        row_end as u8,
    );
}

/// Write a single RGB565 pixel into the current RAMWR stream.
/// Caller must have issued RAMWR (0x2C) and kept CS low.
#[inline(always)]
fn tft_pixel(color: u16) {
    spi_write((color >> 8) as u8);
    spi_write(color as u8);
}

/// ILI9341 minimal init sequence.
fn tft_init() {
    // Software reset
    tft_cmd(0x01);
    // Small delay after reset
    for _ in 0..50_000 { cortex_m::asm::nop(); }

    // Sleep out
    tft_cmd(0x11);
    for _ in 0..50_000 { cortex_m::asm::nop(); }

    // COLMOD: 16 bits/pixel (RGB565 = 0x55)
    tft_cmd1(0x3A, 0x55);

    // Display on
    tft_cmd(0x29);
}

// ---- RGB565 colour constants (EBU colour bar test pattern) ---------------
//
// Colours approximate the standard EBU 75% colour bar order:
//   White | Yellow | Cyan | Green | Magenta | Red | Blue | Black
const WHITE:   u16 = 0xFFFF;
const YELLOW:  u16 = 0xFFE0;
const CYAN:    u16 = 0x07FF;
const GREEN:   u16 = 0x07E0;
const MAGENTA: u16 = 0xF81F;
const RED:     u16 = 0xF800;
const BLUE:    u16 = 0x001F;
const BLACK:   u16 = 0x0000;

/// Draw a 240×16 horizontal band of 8 equal vertical colour bars (30 px each).
/// Each bar is 30 columns wide: bar 0 = col 0..29, bar 1 = col 30..59, etc.
/// Avoids integer division (not available on Cortex-M3) by unrolling with a counter.
fn draw_colour_bars(row_start: u16) {
    const ROWS: u16 = 16;
    const BAR_W: u16 = 30; // 240 / 8 = 30

    let colours = [WHITE, YELLOW, CYAN, GREEN, MAGENTA, RED, BLUE, BLACK];

    tft_set_window(0, 239, row_start, row_start + ROWS - 1);

    cs_low();
    spi_write(0x2C); // RAMWR
    for _row in 0..ROWS {
        for bar_idx in 0..8usize {
            for _px in 0..BAR_W {
                tft_pixel(colours[bar_idx]);
            }
        }
    }
    cs_high();
}

/// Draw a solid-colour 240×16 horizontal band.
fn draw_solid_band(row_start: u16, color: u16) {
    const COLS: u16 = 240;
    const ROWS: u16 = 16;

    tft_set_window(0, COLS - 1, row_start, row_start + ROWS - 1);

    cs_low();
    spi_write(0x2C); // RAMWR
    for _ in 0..(COLS * ROWS) {
        tft_pixel(color);
    }
    cs_high();
}

// --------------------------------------------------------------------------

#[entry]
fn main() -> ! {
    unsafe {
        // Enable RCC for GPIOA (bit 2) and SPI1 (bit 12)
        let apb2enr = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2enr | (1 << 12) | (1 << 2));

        // Configure GPIOA CRL:
        //   PA4 (CS)   = output PP 50 MHz  → bits [19:16] = 0011
        //   PA5 (SCK)  = AF PP 50 MHz      → bits [23:20] = 1011
        //   PA6 (MISO) = input floating    → bits [27:24] = 0100
        //   PA7 (MOSI) = AF PP 50 MHz      → bits [31:28] = 1011
        let mut crl = core::ptr::read_volatile(GPIOA_CRL);
        crl &= 0x0000_FFFF; // clear PA4..PA7 nibbles
        crl |= 0xB4B3_0000;
        core::ptr::write_volatile(GPIOA_CRL, crl);

        // CS idle high
        core::ptr::write_volatile(GPIOA_BSRR, 1 << 4);

        // SPI1: master mode, BR=000 (f/2), CPOL=0, CPHA=0, SPE
        // CR1 = SPE(6) | MSTR(2) = 0x0044
        core::ptr::write_volatile(SPI1_CR1, 0x0044u16);
    }

    uart_str("ILI9341 TFT Lab\n");

    tft_init();
    uart_str("TFT init done\n");

    // Row 0..15: EBU colour bars
    draw_colour_bars(0);
    uart_str("colour bars drawn\n");

    // Row 16..31: solid red
    draw_solid_band(16, RED);
    uart_str("red band drawn\n");

    // Row 32..47: solid green
    draw_solid_band(32, GREEN);
    uart_str("green band drawn\n");

    // Row 48..63: solid blue
    draw_solid_band(48, BLUE);
    uart_str("blue band drawn\n");

    uart_str("frame done\n");

    loop {
        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
        uart_str("running\n");
    }
}
