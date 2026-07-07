#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use cortex_m_rt::entry;
use panic_halt as _;

const I2C1_BASE: u32 = 0x4000_5400;
const GPIOA_BASE: u32 = 0x4001_0800;
const GPIOB_BASE: u32 = 0x4001_0C00;
const RCC_BASE: u32 = 0x4002_1000;
const UART1_BASE: u32 = 0x4001_3800;

const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;
const GPIOA_CRH: *mut u32 = (GPIOA_BASE + 0x04) as *mut u32;
const GPIOB_CRL: *mut u32 = GPIOB_BASE as *mut u32;
const UART1_SR: *const u32 = UART1_BASE as *const u32;
const UART1_DR: *mut u32 = (UART1_BASE + 0x04) as *mut u32;
const UART1_BRR: *mut u32 = (UART1_BASE + 0x08) as *mut u32;
const UART1_CR1: *mut u32 = (UART1_BASE + 0x0C) as *mut u32;
const I2C1_CR1: *mut u32 = (I2C1_BASE + 0x00) as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_OAR1: *mut u32 = (I2C1_BASE + 0x08) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *const u32 = (I2C1_BASE + 0x14) as *const u32;
const I2C1_SR2: *const u32 = (I2C1_BASE + 0x18) as *const u32;
const I2C1_CCR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_TRISE: *mut u32 = (I2C1_BASE + 0x20) as *mut u32;

// SSD1306 at 0x3C → write address 0x78
const OLED_W: u8 = 0x78;

fn uart_byte(byte: u8) {
    unsafe {
        for _ in 0..1024 {
            if core::ptr::read_volatile(UART1_SR) & (1 << 7) != 0 {
                break;
            }
        }
        core::ptr::write_volatile(UART1_DR, byte as u32);
    }
}

fn uart_str(value: &str) {
    for byte in value.bytes() {
        uart_byte(byte);
    }
}

fn i2c_wait(mask: u32) {
    for _ in 0..16_000 {
        let sr1 = unsafe { core::ptr::read_volatile(I2C1_SR1) };
        if sr1 & mask != 0 {
            return;
        }
    }
}

fn board_init() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(
            RCC_APB2ENR,
            apb2 | (1 << 0) | (1 << 2) | (1 << 3) | (1 << 14),
        );
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 21));

        // PA9 = USART1_TX, alternate push-pull 50 MHz.
        let crh = core::ptr::read_volatile(GPIOA_CRH);
        core::ptr::write_volatile(GPIOA_CRH, (crh & !(0xF << 4)) | (0xB << 4));
        // PB6/PB7 = I2C1 SCL/SDA, alternate open-drain 50 MHz.
        let crl = core::ptr::read_volatile(GPIOB_CRL);
        core::ptr::write_volatile(GPIOB_CRL, (crl & !(0xFF << 24)) | (0xFF << 24));

        core::ptr::write_volatile(UART1_BRR, 0x45);
        core::ptr::write_volatile(UART1_CR1, (1 << 13) | (1 << 3));

        core::ptr::write_volatile(I2C1_CR1, 0);
        core::ptr::write_volatile(I2C1_CR2, 8);
        core::ptr::write_volatile(I2C1_CCR, 40);
        core::ptr::write_volatile(I2C1_TRISE, 9);
        core::ptr::write_volatile(I2C1_OAR1, 1 << 14);
        core::ptr::write_volatile(I2C1_CR1, 1);
    }
}

fn i2c_start() {
    unsafe { core::ptr::write_volatile(I2C1_CR1, 0x0001 | 0x0100) }
    i2c_wait(0x0001);
}

fn i2c_stop() {
    unsafe { core::ptr::write_volatile(I2C1_CR1, 0x0001 | 0x0200) }
}

fn i2c_write(byte: u8) {
    unsafe { core::ptr::write_volatile(I2C1_DR, byte as u32) }
    i2c_wait(0x0080);
}

fn i2c_address(byte: u8) {
    unsafe { core::ptr::write_volatile(I2C1_DR, byte as u32) }
    i2c_wait(0x0002);
    unsafe {
        let _ = core::ptr::read_volatile(I2C1_SR1);
        let _ = core::ptr::read_volatile(I2C1_SR2);
    }
}

/// Send a single command byte to SSD1306.
/// Control byte 0x00 = command stream.
fn oled_cmd(cmd: u8) {
    i2c_start();
    i2c_address(OLED_W);
    i2c_write(0x00); // control: command
    i2c_write(cmd);
    i2c_stop();
}

/// Send a command byte followed by one parameter.
fn oled_cmd1(cmd: u8, param: u8) {
    i2c_start();
    i2c_address(OLED_W);
    i2c_write(0x00); // control: command
    i2c_write(cmd);
    i2c_write(param);
    i2c_stop();
}

/// Send a command byte followed by two parameters.
fn oled_cmd2(cmd: u8, p1: u8, p2: u8) {
    i2c_start();
    i2c_address(OLED_W);
    i2c_write(0x00); // control: command
    i2c_write(cmd);
    i2c_write(p1);
    i2c_write(p2);
    i2c_stop();
}

/// Standard SSD1306 initialisation sequence.
fn oled_init() {
    oled_cmd(0xAE); // display off
    oled_cmd1(0xD5, 0x80); // clock div / osc freq
    oled_cmd1(0xA8, 0x3F); // multiplex ratio 64
    oled_cmd1(0xD3, 0x00); // display offset 0
    oled_cmd(0x40); // start line 0
    oled_cmd1(0x8D, 0x14); // charge pump on
    oled_cmd1(0x20, 0x00); // horizontal addressing mode
    oled_cmd(0xA1); // segment remap (col 127 = SEG0)
    oled_cmd(0xC8); // COM scan direction reversed
    oled_cmd1(0xDA, 0x12); // COM pins hardware config
    oled_cmd1(0x81, 0xCF); // contrast
    oled_cmd1(0xD9, 0xF1); // pre-charge period
    oled_cmd1(0xDB, 0x40); // VCOMH deselect level
    oled_cmd(0xA4); // display from RAM (not all-on)
    oled_cmd(0xA6); // normal (non-inverted) display
    oled_cmd(0xAF); // display on
}

/// Fill the entire 128×64 framebuffer with the given page byte (sent as data).
/// page_byte = 0xFF fills all 8 pixels in that page column; 0x00 clears.
fn oled_fill(page_byte: u8) {
    // Set column 0..127, page 0..7
    oled_cmd2(0x21, 0x00, 0x7F);
    oled_cmd2(0x22, 0x00, 0x07);

    i2c_start();
    i2c_address(OLED_W);
    i2c_write(0x40); // control: data stream
    for _ in 0..(128 * 8) {
        i2c_write(page_byte);
    }
    i2c_stop();
}

// 5×7 glyphs for 'L' and 'W' (column-major, bottom = bit 0, 7 rows used of 8).
// Each glyph is 5 columns wide; spacing = 1 empty column.
const GLYPH_L: [u8; 5] = [0x7F, 0x40, 0x40, 0x40, 0x40];
const GLYPH_W: [u8; 5] = [0x3F, 0x40, 0x30, 0x40, 0x3F];

/// Draw a single 5×7 glyph at (start_col, page) in GDDRAM.
/// We use page addressing mode (0x02) for this, re-entering command mode.
fn oled_draw_glyph(start_col: u8, page: u8, glyph: &[u8; 5]) {
    for (i, &col_data) in glyph.iter().enumerate() {
        let col = start_col + i as u8;
        // Set page address
        oled_cmd(0xB0 | (page & 0x07));
        // Set column address (lower nibble)
        oled_cmd(col & 0x0F);
        // Set column address (upper nibble)
        oled_cmd(0x10 | (col >> 4));
        // Write 1 data byte for this column
        i2c_start();
        i2c_address(OLED_W);
        i2c_write(0x40); // data
        i2c_write(col_data);
        i2c_stop();
    }
}

/// Render a "LW" pattern visible in the centre of the display.
/// Steps:
///  1. Clear screen.
///  2. Draw a horizontal white band across pages 2..5 (rows 16..47).
///  3. Draw the letters 'L' and 'W' inside the band using page-mode glyphs.
fn oled_render_demo() {
    // Switch to horizontal mode for the band fill
    oled_cmd1(0x20, 0x00);

    // Clear full screen
    oled_fill(0x00);

    // Fill the centre 4 pages (pages 2, 3, 4, 5) = rows 16..47
    oled_cmd2(0x21, 0x00, 0x7F);
    oled_cmd2(0x22, 0x02, 0x05);
    i2c_start();
    i2c_address(OLED_W);
    i2c_write(0x40);
    for _ in 0..(128 * 4) {
        i2c_write(0xFF);
    }
    i2c_stop();

    // Switch to page addressing for glyphs
    oled_cmd1(0x20, 0x02);

    // Draw 'L' at column 50, page 3 (row 24..31) — sits inside the band
    oled_draw_glyph(50, 3, &GLYPH_L);

    // Draw 'W' at column 60, page 3
    oled_draw_glyph(60, 3, &GLYPH_W);
}

#[entry]
fn main() -> ! {
    board_init();
    uart_str("SSD1306 Hello Lab\n");

    oled_init();
    uart_str("OLED init done\n");

    oled_render_demo();
    uart_str("OLED render done\n");

    loop {
        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
