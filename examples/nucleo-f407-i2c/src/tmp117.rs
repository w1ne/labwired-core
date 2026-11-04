// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// NUCLEO-F407 + TMP117: read the device ID and temperature result through
// the real STM32F407 I2C1 state machine. PA5 goes high only when both words
// match the attached TMP117 model; USART2 reports the same gated result.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_3800;
const GPIOA_BASE: u32 = 0x4002_0000;
const GPIOB_BASE: u32 = 0x4002_0400;
const USART2_BASE: u32 = 0x4000_4400;
const I2C1_BASE: u32 = 0x4000_5400;

const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x40) as *mut u32;

const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;

const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_OTYPER: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;
const GPIOB_OSPEEDR: *mut u32 = (GPIOB_BASE + 0x08) as *mut u32;
const GPIOB_PUPDR: *mut u32 = (GPIOB_BASE + 0x0C) as *mut u32;
const GPIOB_AFRL: *mut u32 = (GPIOB_BASE + 0x20) as *mut u32;

const USART2_SR: *const u32 = USART2_BASE as *const u32;
const USART2_DR: *mut u32 = (USART2_BASE + 0x04) as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x08) as *mut u32;
const USART2_CR1: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;

const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *mut u32 = (I2C1_BASE + 0x14) as *mut u32;
const I2C1_SR2: *const u32 = (I2C1_BASE + 0x18) as *const u32;
const I2C1_CCR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_TRISE: *mut u32 = (I2C1_BASE + 0x20) as *mut u32;

const I2C_CR1_PE: u32 = 1 << 0;
const I2C_CR1_START: u32 = 1 << 8;
const I2C_CR1_STOP: u32 = 1 << 9;
const I2C_CR1_ACK: u32 = 1 << 10;
const I2C_CR1_SWRST: u32 = 1 << 15;

const I2C_SR1_SB: u32 = 1 << 0;
const I2C_SR1_ADDR: u32 = 1 << 1;
const I2C_SR1_BTF: u32 = 1 << 2;
const I2C_SR1_RXNE: u32 = 1 << 6;
const I2C_SR1_TXE: u32 = 1 << 7;

const TMP117_ADDRESS: u8 = 0x48;
const TMP117_DEVICE_ID: u16 = 0x0117;
const TMP117_25C_RAW: u16 = 0x0C80;
const POLL_BUDGET: u32 = 100_000;

#[entry]
fn main() -> ! {
    unsafe {
        init_clocks();
        init_gpio();
        init_uart();
        init_i2c();

        let device_id = tmp117_read_register(0x0F);
        let temperature = tmp117_read_register(0x00);
        let passed = device_id == Some(TMP117_DEVICE_ID) && temperature == Some(TMP117_25C_RAW);

        if passed {
            uart_write(b"TMP117_ID=0x0117 TEMP_RAW=0x0C80 OK\r\n");
            write_volatile(GPIOA_BSRR, 1 << 5);
        } else {
            uart_write(b"TMP117 READ FAIL\r\n");
            write_volatile(GPIOA_BSRR, 1 << (5 + 16));
        }

        loop {
            cortex_m::asm::nop();
        }
    }
}

unsafe fn init_clocks() {
    write_volatile(
        RCC_AHB1ENR,
        read_volatile(RCC_AHB1ENR) | (1 << 0) | (1 << 1),
    );
    write_volatile(
        RCC_APB1ENR,
        read_volatile(RCC_APB1ENR) | (1 << 17) | (1 << 21),
    );
}

unsafe fn init_gpio() {
    // PA2 = USART2 TX (AF7); PA5 = the NUCLEO-F407 user LED.
    let mut moder = read_volatile(GPIOA_MODER);
    moder = (moder & !((0b11 << 4) | (0b11 << 10))) | (0b10 << 4) | (0b01 << 10);
    write_volatile(GPIOA_MODER, moder);
    let mut afrl = read_volatile(GPIOA_AFRL);
    afrl = (afrl & !(0xF << 8)) | (7 << 8);
    write_volatile(GPIOA_AFRL, afrl);

    // PB6/PB7 = I2C1 SCL/SDA (AF4), open-drain with internal pull-ups.
    let mut moder = read_volatile(GPIOB_MODER);
    moder = (moder & !((0b11 << 12) | (0b11 << 14))) | (0b10 << 12) | (0b10 << 14);
    write_volatile(GPIOB_MODER, moder);
    write_volatile(
        GPIOB_OTYPER,
        read_volatile(GPIOB_OTYPER) | (1 << 6) | (1 << 7),
    );
    let mut ospeedr = read_volatile(GPIOB_OSPEEDR);
    ospeedr |= (0b11 << 12) | (0b11 << 14);
    write_volatile(GPIOB_OSPEEDR, ospeedr);
    let mut pupdr = read_volatile(GPIOB_PUPDR);
    pupdr = (pupdr & !((0b11 << 12) | (0b11 << 14))) | (0b01 << 12) | (0b01 << 14);
    write_volatile(GPIOB_PUPDR, pupdr);
    let mut afrl = read_volatile(GPIOB_AFRL);
    afrl = (afrl & !((0xF << 24) | (0xF << 28))) | (4 << 24) | (4 << 28);
    write_volatile(GPIOB_AFRL, afrl);
}

unsafe fn init_uart() {
    // Reset HSI is 16 MHz; BRR 0x008B gives the 115200-baud VCP rate.
    write_volatile(USART2_BRR, 0x008B);
    write_volatile(USART2_CR1, (1 << 13) | (1 << 3));
}

unsafe fn uart_write(bytes: &[u8]) {
    for byte in bytes {
        if !poll(USART2_SR, 1 << 7) {
            return;
        }
        write_volatile(USART2_DR, u32::from(*byte));
    }
}

unsafe fn init_i2c() {
    write_volatile(I2C1_CR1, I2C_CR1_SWRST);
    write_volatile(I2C1_CR1, 0);
    write_volatile(I2C1_CR2, 16);
    write_volatile(I2C1_CCR, 80);
    write_volatile(I2C1_TRISE, 17);
    write_volatile(I2C1_CR1, I2C_CR1_PE);
}

unsafe fn tmp117_read_register(register: u8) -> Option<u16> {
    if !i2c_start() || !i2c_address(TMP117_ADDRESS, false) || !i2c_write_byte(register) {
        return None;
    }
    if !i2c_start() || !i2c_address(TMP117_ADDRESS, true) {
        return None;
    }
    let msb = i2c_read_ack()?;
    let lsb = i2c_read_nack()?;
    Some((u16::from(msb) << 8) | u16::from(lsb))
}

unsafe fn i2c_start() -> bool {
    write_volatile(I2C1_CR1, read_volatile(I2C1_CR1) | I2C_CR1_START);
    poll(I2C1_SR1 as *const u32, I2C_SR1_SB)
}

unsafe fn i2c_address(address: u8, read: bool) -> bool {
    write_volatile(I2C1_DR, u32::from((address << 1) | u8::from(read)));
    if !poll(I2C1_SR1 as *const u32, I2C_SR1_ADDR) {
        return false;
    }
    let _ = read_volatile(I2C1_SR2);
    true
}

unsafe fn i2c_write_byte(byte: u8) -> bool {
    if !poll(I2C1_SR1 as *const u32, I2C_SR1_TXE) {
        return false;
    }
    write_volatile(I2C1_DR, u32::from(byte));
    poll(I2C1_SR1 as *const u32, I2C_SR1_BTF)
}

unsafe fn i2c_read_ack() -> Option<u8> {
    write_volatile(I2C1_CR1, read_volatile(I2C1_CR1) | I2C_CR1_ACK);
    if poll(I2C1_SR1 as *const u32, I2C_SR1_RXNE) {
        Some(read_volatile(I2C1_DR) as u8)
    } else {
        None
    }
}

unsafe fn i2c_read_nack() -> Option<u8> {
    let cr1 = read_volatile(I2C1_CR1);
    write_volatile(I2C1_CR1, (cr1 & !I2C_CR1_ACK) | I2C_CR1_STOP);
    if poll(I2C1_SR1 as *const u32, I2C_SR1_RXNE) {
        Some(read_volatile(I2C1_DR) as u8)
    } else {
        None
    }
}

unsafe fn poll(register: *const u32, mask: u32) -> bool {
    for _ in 0..POLL_BUDGET {
        if read_volatile(register) & mask != 0 {
            return true;
        }
    }
    false
}
