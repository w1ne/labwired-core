#![no_std]
#![no_main]
#![allow(clippy::identity_op, clippy::needless_range_loop)]

//! STM32F103 + DS3231 RTC over I2C1.
//! Reads BCD time registers and prints HH:MM:SS.

use cortex_m_rt::entry;
use panic_halt as _;

const I2C1_BASE: u32 = 0x4000_5400;
const UART1_DR: *mut u8 = (0x4001_3800 + 0x04) as *mut u8;

const I2C1_CR1: *mut u32 = (I2C1_BASE + 0x00) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *const u32 = (I2C1_BASE + 0x14) as *const u32;

const ADDR_W: u8 = 0xD0; // 0x68 << 1
const ADDR_R: u8 = 0xD1;

fn uart_byte(byte: u8) {
    unsafe { core::ptr::write_volatile(UART1_DR, byte) }
}

fn uart_str(value: &str) {
    for byte in value.bytes() {
        uart_byte(byte);
    }
}

fn uart_2digit(n: u8) {
    uart_byte(b'0' + (n / 10));
    uart_byte(b'0' + (n % 10));
}

fn from_bcd(v: u8) -> u8 {
    ((v >> 4) * 10) + (v & 0x0F)
}

fn i2c_wait(mask: u32) {
    for _ in 0..128 {
        let sr1 = unsafe { core::ptr::read_volatile(I2C1_SR1) };
        if sr1 & mask != 0 {
            return;
        }
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

fn i2c_read_byte() -> u8 {
    i2c_wait(0x0040);
    unsafe { core::ptr::read_volatile(I2C1_DR) as u8 }
}

fn ds3231_read_reg(reg: u8) -> u8 {
    i2c_start();
    i2c_write(ADDR_W);
    i2c_write(reg);
    i2c_start();
    i2c_write(ADDR_R);
    let v = i2c_read_byte();
    i2c_stop();
    v
}

#[entry]
fn main() -> ! {
    uart_str("DS3231 RTC Lab\n");

    loop {
        let sec = from_bcd(ds3231_read_reg(0x00) & 0x7F);
        let min = from_bcd(ds3231_read_reg(0x01) & 0x7F);
        let hour = from_bcd(ds3231_read_reg(0x02) & 0x3F);
        uart_str("TIME=");
        uart_2digit(hour);
        uart_byte(b':');
        uart_2digit(min);
        uart_byte(b':');
        uart_2digit(sec);
        uart_byte(b'\n');
        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
