//! ILI9341 TFT Lab — STM32F103
//!
//! Demonstrates the ILI9341 240×320 RGB565 display simulator:
//!   1. SPI1 + PA4 (CS) initialisation
//!   2. ILI9341 init sequence (SLPOUT → COLMOD → DISPON)
//!   3. CASET / PASET window set + RAMWR pixel write
//!   4. Two 240×16 horizontal colour bands visible in the canvas widget:
//!      - Row 0..15  — eight equal-width vertical colour bars (EBU test pattern)
//!      - Row 16..31 — solid bright red (0xF800)
//!   5. Continuous loop printing "frame done" to UART
//!
//! Pin mapping (SPI1 on STM32F103):
//!   PA4  — CS      (GPIO output push-pull)
//!   PA5  — SCK     (AF push-pull)
//!   PA6  — MISO    (input floating — not used by ILI9341 in write-only mode)
//!   PA7  — MOSI    (AF push-pull)
//!   PB0  — D/C     (GPIO output push-pull)

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// ----- Register addresses -------------------------------------------------

const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;
const GPIOA_CRL: *mut u32 = 0x4001_0800 as *mut u32;
// CRH is the same four-bits-per-pin mux as CRL, for PA8..PA15 (PA9 = TX).
const GPIOA_CRH: *mut u32 = 0x4001_0804 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4001_0810 as *mut u32;
// ⚠️ The two `*_BRR` names below are the GPIO **bit-reset** registers, not the
// USART baud divisor — that one is `UART1_BRR` at 0x4001_3808.
const GPIOA_BRR: *mut u32 = 0x4001_0814 as *mut u32;
const GPIOB_CRL: *mut u32 = 0x4001_0C00 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4001_0C10 as *mut u32;
const GPIOB_BRR: *mut u32 = 0x4001_0C14 as *mut u32;
const SPI1_CR1: *mut u16 = 0x4001_3000 as *mut u16;
const SPI1_SR: *const u16 = 0x4001_3008 as *const u16;
const SPI1_DR: *mut u16 = 0x4001_300C as *mut u16;
const UART1_BRR: *mut u32 = (0x4001_3800 + 0x08) as *mut u32;
const UART1_CR1: *mut u32 = (0x4001_3800 + 0x0C) as *mut u32;
const UART1_DR: *mut u8 = (0x4001_3800 + 0x04) as *mut u8;

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
        if sr & 0x0002 != 0 {
            break;
        }
    }
    unsafe { core::ptr::write_volatile(SPI1_DR, byte as u16) };
    // Wait until RXNE (bit 0) so we don't over-fill the TX FIFO
    for _ in 0..2048 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        if sr & 0x0001 != 0 {
            break;
        }
    }
    // Drain RX
    let _ = unsafe { core::ptr::read_volatile(SPI1_DR) };
}

fn cs_low() {
    unsafe { core::ptr::write_volatile(GPIOA_BRR, 1 << 4) }
}
fn cs_high() {
    unsafe { core::ptr::write_volatile(GPIOA_BSRR, 1 << 4) }
}

/// D/C low — the next byte on the wire is a COMMAND.
fn dc_command() {
    unsafe { core::ptr::write_volatile(GPIOB_BRR, 1 << 0) }
}
/// D/C high — the next bytes are parameters or pixel data.
fn dc_data() {
    unsafe { core::ptr::write_volatile(GPIOB_BSRR, 1 << 0) }
}

// ----- ILI9341 protocol ---------------------------------------------------
//
// Framing is the D/C wire, because that is the only thing an ILI9341 in 4-line
// serial mode has: the controller samples D/C on each byte's first clock edge,
// low for a command and high for data (datasheet §7.3.2). It does NOT infer
// framing from chip-select boundaries — there is no mode in which "the first
// byte after CS falls is a command".
//
// This sketch used to leave PB0 undriven and rely on that CS-boundary idea,
// which the simulator's legacy no-D/C path happened to approximate. On the
// hosted lab, whose canvas wires PB0 -> DC, the compiled manifest carries
// `dc_pin` and the model therefore reads framing from the wire like silicon —
// so an undriven PB0 sat low forever, every byte decoded as a command, and the
// panel rendered perfectly blank with no error anywhere. Real hardware would
// have done exactly the same thing.

fn tft_cmd(cmd: u8) {
    cs_low();
    dc_command();
    spi_write(cmd);
    cs_high();
}

fn tft_cmd1(cmd: u8, p0: u8) {
    cs_low();
    dc_command();
    spi_write(cmd);
    dc_data();
    spi_write(p0);
    cs_high();
}

fn tft_cmd4(cmd: u8, p0: u8, p1: u8, p2: u8, p3: u8) {
    cs_low();
    dc_command();
    spi_write(cmd);
    dc_data();
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
    for _ in 0..50_000 {
        cortex_m::asm::nop();
    }

    // Sleep out
    tft_cmd(0x11);
    for _ in 0..50_000 {
        cortex_m::asm::nop();
    }

    // COLMOD: 16 bits/pixel (RGB565 = 0x55)
    tft_cmd1(0x3A, 0x55);

    // Display on
    tft_cmd(0x29);
}

// ---- RGB565 colour constants (EBU colour bar test pattern) ---------------
//
// Colours approximate the standard EBU 75% colour bar order:
//   White | Yellow | Cyan | Green | Magenta | Red | Blue | Black
const WHITE: u16 = 0xFFFF;
const YELLOW: u16 = 0xFFE0;
const CYAN: u16 = 0x07FF;
const GREEN: u16 = 0x07E0;
const MAGENTA: u16 = 0xF81F;
const RED: u16 = 0xF800;
const BLUE: u16 = 0x001F;
const BLACK: u16 = 0x0000;

/// Draw a 240×16 horizontal band of 8 equal vertical colour bars (30 px each).
/// Each bar is 30 columns wide: bar 0 = col 0..29, bar 1 = col 30..59, etc.
/// Avoids integer division (not available on Cortex-M3) by unrolling with a counter.
fn draw_colour_bars(row_start: u16) {
    const ROWS: u16 = 16;
    const BAR_W: u16 = 30; // 240 / 8 = 30

    let colours = [WHITE, YELLOW, CYAN, GREEN, MAGENTA, RED, BLUE, BLACK];

    tft_set_window(0, 239, row_start, row_start + ROWS - 1);

    cs_low();
    dc_command();
    spi_write(0x2C); // RAMWR
    dc_data();
    for _row in 0..ROWS {
        for &color in &colours {
            for _px in 0..BAR_W {
                tft_pixel(color);
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
    dc_command();
    spi_write(0x2C); // RAMWR
    dc_data();
    for _ in 0..(COLS * ROWS) {
        tft_pixel(color);
    }
    cs_high();
}

// --------------------------------------------------------------------------

#[entry]
fn main() -> ! {
    unsafe {
        // Enable RCC for AFIO (bit 0), GPIOA (bit 2), GPIOB (bit 3, the D/C
        // port), SPI1 (bit 12) and USART1 (bit 14).
        //
        // USART1EN was missing here, and USART1 is clock-gated out of reset on
        // this chip (stm32f103.yaml pins its gate to APB2ENR bit 14), so every
        // byte written to its data register was dropped on the gated bus exactly
        // as it is on silicon. The sketch printed nothing at all and the smoke
        // test's three uart_contains assertions had no way to pass.
        let apb2enr = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(
            RCC_APB2ENR,
            apb2enr | (1 << 14) | (1 << 12) | (1 << 3) | (1 << 2) | 1,
        );

        // Configure GPIOA CRL:
        //   PA4 (CS)   = output PP 50 MHz  → bits [19:16] = 0011
        //   PA5 (SCK)  = AF PP 50 MHz      → bits [23:20] = 1011
        //   PA6 (MISO) = input floating    → bits [27:24] = 0100
        //   PA7 (MOSI) = AF PP 50 MHz      → bits [31:28] = 1011
        let mut crl = core::ptr::read_volatile(GPIOA_CRL);
        crl &= 0x0000_FFFF; // clear PA4..PA7 nibbles
        crl |= 0xB4B3_0000;
        core::ptr::write_volatile(GPIOA_CRL, crl);

        // GPIOB CRL: PB0 (D/C) = output PP 50 MHz → bits [3:0] = 0011
        let mut crl_b = core::ptr::read_volatile(GPIOB_CRL);
        crl_b &= 0xFFFF_FFF0;
        crl_b |= 0x0000_0003;
        core::ptr::write_volatile(GPIOB_CRL, crl_b);

        // CS idle high; D/C idles low (command).
        core::ptr::write_volatile(GPIOA_BSRR, 1 << 4);
        core::ptr::write_volatile(GPIOB_BRR, 1 << 0);

        // PA9 = USART1_TX, alternate-function push-pull 50 MHz.
        //
        // PA9 is USART1_TX in the **Default** alternate-function column
        // (DS5319 Rev 20, Table 5, p.31), so no AFIO remap is involved. The CRH
        // nibble for PA9 is bits [7:4]; 0xB = MODE 0b11 (output, 50 MHz) + CNF
        // 0b10 (alternate-function push-pull). Enabling the USART clock and
        // writing DR was enough for LabWired's permissive USART model, but the
        // pin itself stayed a floating input: the pad route never went live, so
        // a logic analyzer on PA9 read the GPIO latch — a flat line — while the
        // transaction-level bus monitor decoded the same traffic fine.
        let crh = core::ptr::read_volatile(GPIOA_CRH);
        core::ptr::write_volatile(GPIOA_CRH, (crh & !(0xF << 4)) | (0xB << 4));

        // USART1 at 115 200 8N1. BRR = f_PCLK2 / baud at the default 16×
        // oversampling; this firmware never touches the PLL, so the part runs
        // on the 8 MHz HSI it selects at reset (DS5319 Rev 20 §2.3.7, p.15):
        // 8_000_000 / 115_200 = 69.44 → 69 = 0x45. A zero BRR means no bit
        // period at all, so there is nothing for a probe to see.
        core::ptr::write_volatile(UART1_BRR, 0x45);
        // USART1: UE (bit 13) | TE (bit 3) — transmit only, no interrupts.
        core::ptr::write_volatile(UART1_CR1, (1 << 13) | (1 << 3));

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
