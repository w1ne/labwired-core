#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

const RCC: u32 = 0x4602_0C00;
const GPIOA: u32 = 0x4202_0000;
const GPIOB: u32 = 0x4202_0400;
const GPIOC: u32 = 0x4202_0800;
const GPIOG: u32 = 0x4202_1800;
const I2C1: u32 = 0x4000_5400;
const USART1: u32 = 0x4001_3800;
const OLED_ADDRESS: u32 = 0x3C;
const WIDTH: usize = 128;
const HEIGHT: usize = 64;
const FRAME_BYTES: usize = WIDTH * HEIGHT / 8;
const LANES: [u8; 3] = [21, 64, 107];

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn rd32(address: u32) -> u32 {
    unsafe { read_volatile(address as *const u32) }
}

fn wr32(address: u32, value: u32) {
    unsafe { write_volatile(address as *mut u32, value) }
}

fn set_field(address: u32, shift: u32, width_mask: u32, value: u32) {
    let value_now = rd32(address);
    wr32(
        address,
        (value_now & !(width_mask << shift)) | ((value & width_mask) << shift),
    );
}

fn configure_output(port: u32, pin: u32) {
    set_field(port, pin * 2, 0x3, 0x1);
    set_field(port + 0x04, pin, 0x1, 0x0);
}

fn configure_i2c_pin(pin: u32) {
    set_field(GPIOB, pin * 2, 0x3, 0x2);
    set_field(GPIOB + 0x04, pin, 0x1, 0x1);
    set_field(GPIOB + 0x0C, pin * 2, 0x3, 0x0);
    let af_shift = (pin - 8) * 4;
    set_field(GPIOB + 0x24, af_shift, 0xF, 0x4);
}

fn configure_uart_pin(pin: u32) {
    set_field(GPIOA, pin * 2, 0x3, 0x2);
    set_field(GPIOA + 0x04, pin, 0x1, 0x0);
    set_field(GPIOA + 0x0C, pin * 2, 0x3, 0x0);
    let af_shift = (pin - 8) * 4;
    set_field(GPIOA + 0x24, af_shift, 0xF, 0x7);
}

fn uart_byte(byte: u8) {
    for _ in 0..10_000 {
        if rd32(USART1 + 0x1C) & (1 << 7) != 0 {
            break;
        }
    }
    unsafe { write_volatile((USART1 + 0x28) as *mut u8, byte) };
}

fn uart_str(text: &str) {
    for byte in text.bytes() {
        uart_byte(byte);
    }
}

fn uart_number(mut value: u32) {
    let mut digits = [0u8; 10];
    let mut used = 0;
    loop {
        digits[used] = b'0' + (value % 10) as u8;
        used += 1;
        value /= 10;
        if value == 0 || used == digits.len() {
            break;
        }
    }
    while used > 0 {
        used -= 1;
        uart_byte(digits[used]);
    }
}

fn board_init() {
    let ahb2 = rd32(RCC + 0x8C);
    wr32(RCC + 0x8C, ahb2 | (1 << 0) | (1 << 1) | (1 << 2) | (1 << 6));
    let apb1 = rd32(RCC + 0x9C);
    wr32(RCC + 0x9C, apb1 | (1 << 21));
    let apb2 = rd32(RCC + 0xA4);
    wr32(RCC + 0xA4, apb2 | (1 << 14));

    configure_output(GPIOB, 7); // LD2 blue
    configure_output(GPIOC, 7); // LD1 green
    configure_output(GPIOG, 2); // LD3 red
    configure_uart_pin(9); // PA9 = USART1_TX, AF7
    configure_uart_pin(10); // PA10 = USART1_RX, AF7
                            // PC13 resets to analog mode; select input mode before reading B1/IDR.
    set_field(GPIOC, 13 * 2, 0x3, 0x0);
    configure_i2c_pin(8); // PB8 = I2C1_SCL, AF4
    configure_i2c_pin(9); // PB9 = I2C1_SDA, AF4

    // USART1 VCP at 115200 baud from the reset MSI clock (4 MHz).
    wr32(USART1 + 0x0C, 35);
    wr32(USART1, (1 << 0) | (1 << 2) | (1 << 3)); // UE | RE | TE

    // At the reset 4 MHz I2C kernel clock: PRESC=0, SCLDEL=1, SDADEL=1,
    // SCLH=3 and SCLL=5. This is a 400 kHz fast-mode transfer.
    wr32(I2C1 + 0x10, 0x0011_0305);
    wr32(I2C1, 1); // CR1.PE
}

/// A bounded register-level master write to the simulated I2C slave.
fn i2c_write(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > 255 {
        return false;
    }
    wr32(I2C1 + 0x1C, (1 << 4) | (1 << 5)); // Clear stale NACKF/STOPF.
    let cr2 = (OLED_ADDRESS << 1)
        | ((bytes.len() as u32) << 16)
        | (1 << 25) // AUTOEND
        | (1 << 13); // START
    wr32(I2C1 + 0x04, cr2);

    for &byte in bytes {
        let mut ready = false;
        for _ in 0..100_000 {
            let status = rd32(I2C1 + 0x18);
            if status & (1 << 4) != 0 {
                wr32(I2C1 + 0x1C, (1 << 4) | (1 << 5));
                return false;
            }
            if status & (1 << 1) != 0 {
                // TXIS
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        unsafe { write_volatile((I2C1 + 0x28) as *mut u8, byte) };
    }

    for _ in 0..100_000 {
        let status = rd32(I2C1 + 0x18);
        if status & (1 << 4) != 0 {
            wr32(I2C1 + 0x1C, (1 << 4) | (1 << 5));
            return false;
        }
        if status & (1 << 5) != 0 {
            // STOPF
            wr32(I2C1 + 0x1C, 1 << 5);
            return true;
        }
    }
    false
}

fn oled_init() -> bool {
    // Page addressing mode matches the renderer below (one 128-byte write per page).
    i2c_write(&[
        0x00, 0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x02, 0xA1, 0xC8,
        0xDA, 0x12, 0x81, 0xCF, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6, 0xAF,
    ])
}

fn pixel(frame: &mut [u8; FRAME_BYTES], x: i32, y: i32) {
    if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
        return;
    }
    let index = (y as usize / 8) * WIDTH + x as usize;
    frame[index] |= 1 << (y as usize & 7);
}

fn glyph_rows(ch: u8) -> [u8; 7] {
    match ch {
        b'A' => [14, 17, 17, 31, 17, 17, 17],
        b'B' => [30, 17, 17, 30, 17, 17, 30],
        b'C' => [14, 17, 16, 16, 16, 17, 14],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'H' => [17, 17, 17, 31, 17, 17, 17],
        b'M' => [17, 27, 21, 21, 17, 17, 17],
        b'O' => [14, 17, 17, 17, 17, 17, 14],
        b'R' => [30, 17, 17, 30, 20, 18, 17],
        b'S' => [15, 16, 16, 14, 1, 1, 30],
        b'T' => [31, 4, 4, 4, 4, 4, 4],
        b'V' => [17, 17, 17, 17, 17, 10, 4],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b':' => [0, 4, 4, 0, 4, 4, 0],
        b'-' => [0, 0, 0, 31, 0, 0, 0],
        _ => [0; 7],
    }
}

fn text(frame: &mut [u8; FRAME_BYTES], mut x: i32, y: i32, value: &[u8]) {
    for &ch in value {
        let rows = glyph_rows(ch);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    pixel(frame, x + col, y + row as i32);
                }
            }
        }
        x += 6;
    }
}

fn draw_star(frame: &mut [u8; FRAME_BYTES], x: i32, y: i32) {
    for d in -3..=3 {
        pixel(frame, x + d, y);
        pixel(frame, x, y + d);
    }
    pixel(frame, x - 2, y - 2);
    pixel(frame, x + 2, y - 2);
    pixel(frame, x - 2, y + 2);
    pixel(frame, x + 2, y + 2);
}

fn render(frame: &mut [u8; FRAME_BYTES], catcher: u8, star_lane: u8, star_y: u8, score: u32) {
    frame.fill(0);
    text(frame, 28, 2, b"STAR CATCHER");
    for y in 13..15 {
        for x in 4..124 {
            pixel(frame, x, y);
        }
    }
    for y in 18..53 {
        pixel(frame, 42, y);
        pixel(frame, 85, y);
    }
    draw_star(frame, LANES[star_lane as usize] as i32, star_y as i32);
    let x = LANES[catcher as usize] as i32;
    for d in -8..=8 {
        pixel(frame, x + d, 54);
    }
    pixel(frame, x - 7, 52);
    pixel(frame, x + 7, 52);
    text(frame, 5, 56, b"SCORE:");
    let hundreds = ((score / 100) % 10) as u8 + b'0';
    let tens = ((score / 10) % 10) as u8 + b'0';
    let units = (score % 10) as u8 + b'0';
    text(frame, 47, 56, &[hundreds, tens, units]);
    text(frame, 82, 56, b"B1:MOVE");
}

fn write_frame(frame: &[u8; FRAME_BYTES], first_page: usize) -> bool {
    for page in first_page..8 {
        if !i2c_write(&[0x00, 0xB0 | page as u8, 0x00, 0x10]) {
            return false;
        }
        let mut data = [0u8; 129];
        data[0] = 0x40;
        let start = page * WIDTH;
        data[1..].copy_from_slice(&frame[start..start + WIDTH]);
        if !i2c_write(&data) {
            return false;
        }
    }
    true
}

fn set_leds(score: u32) {
    for (pin, bit) in [(7u32, 0), (7, 1), (2, 2)] {
        let port = match bit {
            0 => GPIOC,
            1 => GPIOB,
            _ => GPIOG,
        };
        let bsrr = if score & (1 << bit) != 0 {
            1 << pin
        } else {
            1 << (pin + 16)
        };
        wr32(port + 0x18, bsrr);
    }
}

fn delay() {
    for _ in 0..250_000 {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

fn main() -> ! {
    board_init();
    uart_str("U575 STAR CATCHER\n");
    if (rd32(GPIOC) >> (13 * 2)) & 0x3 != 0 {
        uart_str("B1 INPUT INIT ERROR\n");
        loop {
            delay();
        }
    }
    uart_str("B1 INPUT READY\n");
    if !oled_init() {
        uart_str("OLED I2C ERROR\n");
        loop {
            delay();
        }
    }

    let mut frame = [0u8; FRAME_BYTES];
    let mut catcher = 1u8;
    let mut star_lane = 1u8;
    let mut star_y = 22u8;
    let mut score = 0u32;
    let mut previous_button = false;
    render(&mut frame, catcher, star_lane, star_y, score);
    if !write_frame(&frame, 0) {
        uart_str("OLED FRAME ERROR\n");
        loop {
            delay();
        }
    }
    uart_str("STAR CATCHER READY\n");
    set_leds(score);

    loop {
        delay();
        let button = rd32(GPIOC + 0x10) & (1 << 13) != 0;
        if button && !previous_button {
            catcher = (catcher + 1) % 3;
            uart_str("B1 MOVE\n");
        }
        previous_button = button;

        star_y = star_y.saturating_add(5);
        if star_y >= 50 {
            if catcher == star_lane {
                score = score.saturating_add(1);
                uart_str("CATCH SCORE ");
                uart_number(score);
                uart_str("\n");
            } else {
                uart_str("MISS\n");
            }
            star_lane = (star_lane + 1) % 3;
            star_y = 22;
            set_leds(score);
        }
        render(&mut frame, catcher, star_lane, star_y, score);
        if !write_frame(&frame, 2) {
            uart_str("OLED FRAME ERROR\n");
        }
    }
}
