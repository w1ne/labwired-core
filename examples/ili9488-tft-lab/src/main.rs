//! ILI9488 TFT Lab — STM32F103
//!
//! Boots SPI1 + PA4 (CS) / PB0 (D/C) / PB1 (RESET), runs the ILI9488 init
//! sequence with COLMOD 0x66 (18-bit / three bytes per pixel), and paints a
//! full-frame cyan fill so io-smoke can assert rgb888 colour evidence.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;
const GPIOA_CRL: *mut u32 = 0x4001_0800 as *mut u32;
const GPIOA_CRH: *mut u32 = 0x4001_0804 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4001_0810 as *mut u32;
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

fn uart_byte(b: u8) {
    unsafe { core::ptr::write_volatile(UART1_DR, b) }
}

fn uart_str(s: &str) {
    for b in s.bytes() {
        uart_byte(b);
    }
}

fn spi_write(byte: u8) {
    for _ in 0..2048 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        if sr & 0x0002 != 0 {
            break;
        }
    }
    unsafe { core::ptr::write_volatile(SPI1_DR, byte as u16) };
    for _ in 0..2048 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        if sr & 0x0001 != 0 {
            break;
        }
    }
    let _ = unsafe { core::ptr::read_volatile(SPI1_DR) };
}

fn cs_low() {
    unsafe { core::ptr::write_volatile(GPIOA_BRR, 1 << 4) }
}
fn cs_high() {
    unsafe { core::ptr::write_volatile(GPIOA_BSRR, 1 << 4) }
}
fn dc_command() {
    unsafe { core::ptr::write_volatile(GPIOB_BRR, 1 << 0) }
}
fn dc_data() {
    unsafe { core::ptr::write_volatile(GPIOB_BSRR, 1 << 0) }
}
fn reset_low() {
    unsafe { core::ptr::write_volatile(GPIOB_BRR, 1 << 1) }
}
fn reset_high() {
    unsafe { core::ptr::write_volatile(GPIOB_BSRR, 1 << 1) }
}

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

fn tft_set_window(col_start: u16, col_end: u16, row_start: u16, row_end: u16) {
    tft_cmd4(
        0x2A,
        (col_start >> 8) as u8,
        col_start as u8,
        (col_end >> 8) as u8,
        col_end as u8,
    );
    tft_cmd4(
        0x2B,
        (row_start >> 8) as u8,
        row_start as u8,
        (row_end >> 8) as u8,
        row_end as u8,
    );
}

/// Write one pixel as 18-bit serial RGB666 (three wire bytes), the same packing
/// Adafruit / Nano Pong use after COLMOD 0x66.
#[inline(always)]
fn tft_pixel666(color: u16) {
    spi_write(((color >> 8) & 0xf8) as u8);
    spi_write(((color >> 3) & 0xfc) as u8);
    spi_write(((color << 3) & 0xf8) as u8);
}

fn tft_init() {
    // Hardware reset pulse (active-low).
    reset_low();
    for _ in 0..20_000 {
        cortex_m::asm::nop();
    }
    reset_high();
    for _ in 0..50_000 {
        cortex_m::asm::nop();
    }

    tft_cmd(0x01);
    for _ in 0..50_000 {
        cortex_m::asm::nop();
    }
    tft_cmd(0x11);
    for _ in 0..50_000 {
        cortex_m::asm::nop();
    }
    // COLMOD: 18 bits/pixel (RGB666). Datasheet uses low nibble 0x6; 0x66 is
    // the value Nano Pong and common Arduino drivers write.
    tft_cmd1(0x3A, 0x66);
    tft_cmd(0x29);
}

const CYAN: u16 = 0x07FF;

/// Fill the whole 320×480 frame. serial_color withholds artifact bytes while
/// any pixel is still unknown, so a partial band cannot feed display_region.
fn draw_full_cyan() {
    const COLS: u16 = 320;
    const ROWS: u16 = 480;
    tft_set_window(0, COLS - 1, 0, ROWS - 1);
    cs_low();
    dc_command();
    spi_write(0x2C);
    dc_data();
    let mut n = 0u32;
    while n < (COLS as u32) * (ROWS as u32) {
        tft_pixel666(CYAN);
        n += 1;
    }
    cs_high();
}

#[entry]
fn main() -> ! {
    unsafe {
        let apb2enr = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(
            RCC_APB2ENR,
            apb2enr | (1 << 14) | (1 << 12) | (1 << 3) | (1 << 2) | 1,
        );

        let mut crl = core::ptr::read_volatile(GPIOA_CRL);
        crl &= 0x0000_FFFF;
        crl |= 0xB4B3_0000;
        core::ptr::write_volatile(GPIOA_CRL, crl);

        // PB0 D/C + PB1 RESET = output PP 50 MHz
        let mut crl_b = core::ptr::read_volatile(GPIOB_CRL);
        crl_b &= 0xFFFF_FF00;
        crl_b |= 0x0000_0033;
        core::ptr::write_volatile(GPIOB_CRL, crl_b);

        core::ptr::write_volatile(GPIOA_BSRR, 1 << 4);
        core::ptr::write_volatile(GPIOB_BRR, 1 << 0);
        core::ptr::write_volatile(GPIOB_BSRR, 1 << 1); // RESET idle high

        let crh = core::ptr::read_volatile(GPIOA_CRH);
        core::ptr::write_volatile(GPIOA_CRH, (crh & !(0xF << 4)) | (0xB << 4));

        core::ptr::write_volatile(UART1_BRR, 0x45);
        core::ptr::write_volatile(UART1_CR1, (1 << 13) | (1 << 3));
        core::ptr::write_volatile(SPI1_CR1, 0x0044u16);
    }

    uart_str("ILI9488 TFT Lab\n");
    tft_init();
    uart_str("COLMOD 18-bit\n");
    draw_full_cyan();
    uart_str("frame done\n");

    loop {
        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
        uart_str("running\n");
    }
}
