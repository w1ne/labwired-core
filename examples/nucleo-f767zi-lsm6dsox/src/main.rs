// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// NUCLEO-F767ZI + ST LSM6DSOX: read WHO_AM_I, enable 416 Hz / ±2 g
// acceleration, wait for XLDA, then burst-read signed XYZ over I2C1.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_3800;
const GPIOB_BASE: u32 = 0x4002_0400;
const GPIOD_BASE: u32 = 0x4002_0C00;
const USART3_BASE: u32 = 0x4000_4800;
const I2C1_BASE: u32 = 0x4000_5400;

const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x40) as *mut u32;

const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_OTYPER: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;
const GPIOB_OSPEEDR: *mut u32 = (GPIOB_BASE + 0x08) as *mut u32;
const GPIOB_PUPDR: *mut u32 = (GPIOB_BASE + 0x0C) as *mut u32;
const GPIOB_ODR: *const u32 = (GPIOB_BASE + 0x14) as *const u32;
const GPIOB_BSRR: *mut u32 = (GPIOB_BASE + 0x18) as *mut u32;
const GPIOB_AFRH: *mut u32 = (GPIOB_BASE + 0x24) as *mut u32;

const GPIOD_MODER: *mut u32 = GPIOD_BASE as *mut u32;
const GPIOD_AFRH: *mut u32 = (GPIOD_BASE + 0x24) as *mut u32;

const USART3_SR: *const u32 = USART3_BASE as *const u32;
const USART3_DR: *mut u32 = (USART3_BASE + 0x04) as *mut u32;
const USART3_BRR: *mut u32 = (USART3_BASE + 0x08) as *mut u32;
const USART3_CR1: *mut u32 = (USART3_BASE + 0x0C) as *mut u32;

const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_TIMINGR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_ISR: *const u32 = (I2C1_BASE + 0x18) as *const u32;
const I2C1_ICR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_RXDR: *const u32 = (I2C1_BASE + 0x24) as *const u32;
const I2C1_TXDR: *mut u32 = (I2C1_BASE + 0x28) as *mut u32;

const LSM6DSOX_ADDRESS: u8 = 0x6A;
const WHO_AM_I: u8 = 0x0F;
const CTRL1_XL: u8 = 0x10;
const CTRL1_XL_416_HZ_2_G: u8 = 0x60;
const STATUS_REG: u8 = 0x1E;
const XLDA: u8 = 1;
const OUTX_L_A: u8 = 0x28;

const TXIS: u32 = 1 << 1;
const RXNE: u32 = 1 << 2;
const NACKF: u32 = 1 << 4;
const STOPF: u32 = 1 << 5;
const TC: u32 = 1 << 6;
const START: u32 = 1 << 13;
const RD_WRN: u32 = 1 << 10;
const AUTOEND: u32 = 1 << 25;
const POLL_BUDGET: u32 = 100_000;

#[entry]
fn main() -> ! {
    unsafe {
        init_clocks();
        init_gpio();
        init_console();
        init_i2c();

        let who = read_register(WHO_AM_I);
        let configured_write = write_register(CTRL1_XL, CTRL1_XL_416_HZ_2_G);
        let configured_readback = read_register(CTRL1_XL);
        let ready = wait_for_accelerometer_data();
        let axes = read_accelerometer_xyz();

        if who == Some(0x6C)
            && configured_write
            && configured_readback == Some(CTRL1_XL_416_HZ_2_G)
            && ready
            && axes == Some((1000, -2000, 4000))
        {
            uart_write(b"LSM6DSOX_ID=0x6C XYZ=1000,-2000,4000 PASS\r\n");
            write_volatile(GPIOB_BSRR, 1); // PB0: NUCLEO-F767ZI LD1, active high
        } else if who != Some(0x6C) {
            uart_write(b"LSM6DSOX_FAIL_WHOAMI\r\n");
        } else if !configured_write {
            uart_write(b"LSM6DSOX_FAIL_CONFIG_WRITE\r\n");
        } else if configured_readback != Some(CTRL1_XL_416_HZ_2_G) {
            uart_write(b"LSM6DSOX_FAIL_CONFIG_READBACK\r\n");
        } else if !ready {
            uart_write(b"LSM6DSOX_FAIL_DATA_READY\r\n");
        } else {
            uart_write(b"LSM6DSOX_FAIL_XYZ=");
            if let Some((x, y, z)) = axes {
                uart_write_hex16(x as u16);
                uart_write(b",");
                uart_write_hex16(y as u16);
                uart_write(b",");
                uart_write_hex16(z as u16);
            } else {
                uart_write(b"NONE");
            }
            uart_write(b"\r\n");
        }

        loop {
            cortex_m::asm::nop();
        }
    }
}

unsafe fn init_clocks() {
    // GPIOB (I2C1 + LD1), GPIOD (USART3 VCP), I2C1 and USART3.
    write_volatile(
        RCC_AHB1ENR,
        read_volatile(RCC_AHB1ENR) | (1 << 1) | (1 << 3),
    );
    write_volatile(
        RCC_APB1ENR,
        read_volatile(RCC_APB1ENR) | (1 << 18) | (1 << 21),
    );
}

unsafe fn init_gpio() {
    // PB0 drives the board's active-high LD1.
    let moder = read_volatile(GPIOB_MODER);
    write_volatile(GPIOB_MODER, (moder & !0x3) | 0x1);

    // PB8/PB9 are I2C1 SCL/SDA (AF4), open drain with pull-ups.
    let mut moder = read_volatile(GPIOB_MODER);
    moder = (moder & !((0x3 << 16) | (0x3 << 18))) | (0x2 << 16) | (0x2 << 18);
    write_volatile(GPIOB_MODER, moder);
    write_volatile(
        GPIOB_OTYPER,
        read_volatile(GPIOB_OTYPER) | (1 << 8) | (1 << 9),
    );
    write_volatile(
        GPIOB_OSPEEDR,
        read_volatile(GPIOB_OSPEEDR) | (0x3 << 16) | (0x3 << 18),
    );
    let pupdr = read_volatile(GPIOB_PUPDR);
    write_volatile(
        GPIOB_PUPDR,
        (pupdr & !((0x3 << 16) | (0x3 << 18))) | (0x1 << 16) | (0x1 << 18),
    );
    let afrh = read_volatile(GPIOB_AFRH);
    write_volatile(GPIOB_AFRH, (afrh & !0xFF) | 0x44); // AF4 on PB8/PB9
}

unsafe fn init_console() {
    // PD8/PD9 are USART3 TX/RX (AF7), connected to the ST-LINK VCP.
    let moder = read_volatile(GPIOD_MODER);
    write_volatile(
        GPIOD_MODER,
        (moder & !((0x3 << 16) | (0x3 << 18))) | (0x2 << 16) | (0x2 << 18),
    );
    let afrh = read_volatile(GPIOD_AFRH);
    write_volatile(GPIOD_AFRH, (afrh & !0xFF) | 0x77);
    write_volatile(USART3_BRR, 0x0683); // 9600 baud from the reset HSI16 clock
    write_volatile(USART3_CR1, (1 << 13) | (1 << 3)); // UE | TE
}

unsafe fn init_i2c() {
    // F7 I2C v2 controller. The programmed timing also keeps the transaction
    // cadence deterministic in the simulator; the firmware leaves the RCC on
    // the reset HSI clock rather than changing the system clock tree.
    write_volatile(I2C1_TIMINGR, 0x3000_0F13);
    write_volatile(I2C1_CR1, 1); // PE
}

unsafe fn wait_isr(mask: u32) -> bool {
    for _ in 0..POLL_BUDGET {
        let status = read_volatile(I2C1_ISR);
        if status & NACKF != 0 {
            write_volatile(I2C1_ICR, NACKF);
            return false;
        }
        if status & mask != 0 {
            return true;
        }
    }
    false
}

unsafe fn begin_write(byte_count: u8, autoend: bool) -> bool {
    let mut cr2 = (u32::from(LSM6DSOX_ADDRESS) << 1) | (u32::from(byte_count) << 16) | START;
    if autoend {
        cr2 |= AUTOEND;
    }
    write_volatile(I2C1_CR2, cr2);
    true
}

unsafe fn write_register(register: u8, value: u8) -> bool {
    if !begin_write(2, true) {
        return false;
    }
    if !wait_isr(TXIS) {
        return false;
    }
    write_volatile(I2C1_TXDR, u32::from(register));
    if !wait_isr(TXIS) {
        return false;
    }
    write_volatile(I2C1_TXDR, u32::from(value));
    if !wait_isr(STOPF) {
        return false;
    }
    write_volatile(I2C1_ICR, STOPF);
    true
}

unsafe fn read_register(register: u8) -> Option<u8> {
    let mut value = [0u8; 1];
    if read_bytes(register, &mut value) {
        Some(value[0])
    } else {
        None
    }
}

unsafe fn read_bytes(register: u8, bytes: &mut [u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }

    // Write the register pointer, then issue a repeated START for the read.
    if !begin_write(1, false) || !wait_isr(TXIS) {
        return false;
    }
    write_volatile(I2C1_TXDR, u32::from(register));
    if !wait_isr(TC) {
        return false;
    }

    let cr2 = (u32::from(LSM6DSOX_ADDRESS) << 1)
        | (u32::from(bytes.len() as u8) << 16)
        | RD_WRN
        | AUTOEND
        | START;
    write_volatile(I2C1_CR2, cr2);
    for byte in bytes.iter_mut() {
        if !wait_isr(RXNE) {
            return false;
        }
        *byte = read_volatile(I2C1_RXDR) as u8;
    }
    if !wait_isr(STOPF) {
        return false;
    }
    write_volatile(I2C1_ICR, STOPF);
    true
}

unsafe fn wait_for_accelerometer_data() -> bool {
    for _ in 0..POLL_BUDGET {
        match read_register(STATUS_REG) {
            Some(status) if status & XLDA != 0 => return true,
            Some(_) => {}
            None => return false,
        }
    }
    false
}

unsafe fn read_accelerometer_xyz() -> Option<(i16, i16, i16)> {
    let mut bytes = [0u8; 6];
    if !read_bytes(OUTX_L_A, &mut bytes) {
        return None;
    }
    Some((
        i16::from_le_bytes([bytes[0], bytes[1]]),
        i16::from_le_bytes([bytes[2], bytes[3]]),
        i16::from_le_bytes([bytes[4], bytes[5]]),
    ))
}

unsafe fn uart_write(bytes: &[u8]) {
    for byte in bytes {
        for _ in 0..POLL_BUDGET {
            if read_volatile(USART3_SR) & (1 << 7) != 0 {
                write_volatile(USART3_DR, u32::from(*byte));
                break;
            }
        }
    }
}

unsafe fn uart_write_hex16(value: u16) {
    for shift in (0..4).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        let byte = if nibble < 10 {
            b'0' + nibble
        } else {
            b'A' + (nibble - 10)
        };
        uart_write(&[byte]);
    }
}
