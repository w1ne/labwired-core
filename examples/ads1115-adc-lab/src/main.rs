#![no_std]
#![no_main]
#![allow(clippy::identity_op, clippy::needless_range_loop)]

//! STM32F103 + ADS1115 16-bit ADC over I2C1.
//! Configures single-ended AIN0 and prints raw conversion counts.

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;

/// Enable USART1 (APB2 bit 14) and I2C1 (APB1 bit 21). Required now that
/// stm32f103.yaml clocks those peripherals — unclocked MMIO is dropped.
fn enable_peripheral_clocks() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 14)); // USART1EN
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 21)); // I2C1EN
    }
}


const I2C1_BASE: u32 = 0x4000_5400;
const UART1_DR: *mut u8 = (0x4001_3800 + 0x04) as *mut u8;

const I2C1_CR1: *mut u32 = (I2C1_BASE + 0x00) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *const u32 = (I2C1_BASE + 0x14) as *const u32;

const ADDR_W: u8 = 0x90; // 0x48 << 1
const ADDR_R: u8 = 0x91;

fn uart_byte(byte: u8) {
    unsafe { core::ptr::write_volatile(UART1_DR, byte) }
}

fn uart_str(value: &str) {
    for byte in value.bytes() {
        uart_byte(byte);
    }
}

fn uart_i32(value: i32) {
    if value < 0 {
        uart_byte(b'-');
    }
    let mut n = if value < 0 {
        (-value) as u32
    } else {
        value as u32
    };
    let mut buf = [0u8; 10];
    let mut len = 0;
    loop {
        buf[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    while len > 0 {
        len -= 1;
        uart_byte(buf[len]);
    }
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

fn ads_write_u16(reg: u8, value: u16) {
    i2c_start();
    i2c_write(ADDR_W);
    i2c_write(reg);
    i2c_write((value >> 8) as u8);
    i2c_write((value & 0xFF) as u8);
    i2c_stop();
}

fn ads_read_u16(reg: u8) -> u16 {
    i2c_start();
    i2c_write(ADDR_W);
    i2c_write(reg);
    i2c_start();
    i2c_write(ADDR_R);
    let hi = i2c_read_byte();
    let lo = i2c_read_byte();
    i2c_stop();
    ((hi as u16) << 8) | (lo as u16)
}

#[entry]
fn main() -> ! {
    
    enable_peripheral_clocks();
uart_str("ADS1115 ADC Lab\n");
    // OS=1, MUX=AIN0 single-ended (100), PGA=±4.096 (001), continuous
    ads_write_u16(0x01, 0xC383);

    loop {
        let raw = ads_read_u16(0x00) as i16;
        uart_str("A0_raw=");
        uart_i32(raw as i32);
        uart_byte(b'\n');
        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
