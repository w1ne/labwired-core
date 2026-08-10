// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;

/// GPIOA CRH — the F1 pad mux for PA8..PA15. Four bits per pin, MODE[1:0] then
/// CNF[1:0]. This family has no `MODER` and no `AFR`, so there is no AF number
/// to write: the alternate function of a pin is fixed by the pin.
const GPIOA_CRH: *mut u32 = (0x4001_0800 + 0x04) as *mut u32;

/// Enable AFIO (APB2 bit 0), GPIOA (bit 2), USART1 (bit 14) and I2C1 (APB1
/// bit 21). Required now that stm32f103.yaml clocks those peripherals —
/// unclocked MMIO is dropped.
///
/// AFIOEN and IOPAEN were missing. Without IOPAEN the GPIO port is held in
/// reset, so `usart1_init`'s `CRH` write would be swallowed and PA9 would stay
/// the floating input it is after reset.
fn enable_peripheral_clocks() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        // AFIOEN | IOPAEN | USART1EN
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 0) | (1 << 2) | (1 << 14));
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 21)); // I2C1EN
    }
}

/// Mux PA9 to `USART1_TX` and give the transmitter a baud divisor.
///
/// Clocking USART1 and writing `DR` is all this lab used to do, and it is
/// enough for LabWired's permissive USART model. It is not enough for silicon
/// and it is not enough for a probe: PA9 stays a floating input until `CRH`
/// selects an alternate-function output, so the pad route never goes live and a
/// logic analyzer on PA9 reads the GPIO latch — a flat line — while the
/// transaction-level bus monitor decodes the same traffic fine. A zero `BRR` is
/// the other half: with no divisor there is no bit period, so there is nothing
/// to narrate even once the route exists.
///
/// * PA9 = `USART1_TX` in the **Default** alternate-function column
///   (DS5319 Rev 20, Table 5, p.31), so no AFIO remap is involved.
/// * The `CRH` nibble for PA9 is bits [7:4]. `0xB` = MODE `0b11` (output,
///   50 MHz) + CNF `0b10` (alternate-function push-pull).
/// * `BRR` = f_PCLK2 / baud at the default 16× oversampling. This firmware
///   never touches the PLL, so the part runs on the 8 MHz HSI it selects at
///   reset (DS5319 Rev 20 §2.3.7, p.15): 8_000_000 / 115_200 = 69.44 → 69,
///   i.e. 0x45, which is 115 942 baud (0.6% fast, well inside a UART's budget).
/// * `CR1` = UE (bit 13) | TE (bit 3): transmit only, no interrupts.
fn usart1_init() {
    unsafe {
        let crh = core::ptr::read_volatile(GPIOA_CRH);
        core::ptr::write_volatile(GPIOA_CRH, (crh & !(0xF << 4)) | (0xB << 4));
        core::ptr::write_volatile(UART1_BRR, 0x45);
        core::ptr::write_volatile(UART1_CR1, (1 << 13) | (1 << 3));
    }
}

const I2C1_BASE: u32 = 0x4000_5400;
const UART1_DR: *mut u8 = (0x4001_3800 + 0x04) as *mut u8;
const UART1_BRR: *mut u32 = (0x4001_3800 + 0x08) as *mut u32;
const UART1_CR1: *mut u32 = (0x4001_3800 + 0x0C) as *mut u32;

const I2C1_CR1: *mut u32 = (I2C1_BASE + 0x00) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *const u32 = (I2C1_BASE + 0x14) as *const u32;

// VL53L1X 7-bit address 0x29 → write 0x52, read 0x53
const VL_W: u8 = 0x52;
const VL_R: u8 = 0x53;

// Proximity threshold (mm): readings at or below this report NEAR, otherwise FAR.
// 300 mm is a sensible "something is close" trip point for a tabletop ToF sensor.
const PROXIMITY_THRESHOLD_MM: u16 = 300;

// 16-bit registers (verified against pololu/vl53l1x-arduino)
const REG_GPIO_TIO_HV_STATUS: u16 = 0x0031;
const REG_SYSTEM_MODE_START: u16 = 0x0087;
const REG_RESULT_RANGE_STATUS: u16 = 0x0089;
const REG_RESULT_RANGE_MM: u16 = 0x0096;
const REG_MODEL_ID: u16 = 0x010F;

fn uart_byte(byte: u8) {
    unsafe { core::ptr::write_volatile(UART1_DR, byte) }
}

fn uart_str(value: &str) {
    for byte in value.bytes() {
        uart_byte(byte);
    }
}

fn uart_hex_u8(value: u8) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    uart_byte(HEX[(value >> 4) as usize]);
    uart_byte(HEX[(value & 0xF) as usize]);
}

fn uart_hex_u16(value: u16) {
    uart_hex_u8((value >> 8) as u8);
    uart_hex_u8(value as u8);
}

fn uart_u16(mut n: u16) {
    let mut buf = [0u8; 5];
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

// VL53L1X uses a 16-bit register pointer: write the high byte then the low byte
// before the data byte / repeated-START read.
fn vl_write_register(reg: u16, value: u8) {
    i2c_start();
    i2c_write(VL_W);
    i2c_write((reg >> 8) as u8);
    i2c_write((reg & 0xFF) as u8);
    i2c_write(value);
    i2c_stop();
}

fn vl_read_register(reg: u16) -> u8 {
    i2c_start();
    i2c_write(VL_W);
    i2c_write((reg >> 8) as u8);
    i2c_write((reg & 0xFF) as u8);
    i2c_start(); // repeated START for the read phase
    i2c_write(VL_R);
    let value = i2c_read_byte();
    i2c_stop();
    value
}

fn vl_read_register16(reg: u16) -> u16 {
    let hi = vl_read_register(reg) as u16;
    let lo = vl_read_register(reg + 1) as u16;
    (hi << 8) | lo
}

#[entry]
fn main() -> ! {
    enable_peripheral_clocks();
    usart1_init();
    uart_str("VL53L1X ToF Lab\n");

    // init(): identity check — readReg16Bit(MODEL_ID) must equal 0xEACC.
    let model_id = vl_read_register16(REG_MODEL_ID);
    uart_str("MODEL_ID=0x");
    uart_hex_u16(model_id);
    if model_id == 0xEACC {
        uart_str(" OK\n");
    } else {
        uart_str(" ERR\n");
    }

    // startContinuous(): write 0x40 to SYSTEM__MODE_START.
    vl_write_register(REG_SYSTEM_MODE_START, 0x40);

    // dataReady(): (GPIO__TIO_HV_STATUS & 0x01) == 0.
    let ready = (vl_read_register(REG_GPIO_TIO_HV_STATUS) & 0x01) == 0;
    if ready {
        uart_str("DATA_READY OK\n");
    } else {
        uart_str("DATA_READY ERR\n");
    }

    // Continuous proximity monitor: each iteration reads the live range and
    // classifies it against PROXIMITY_THRESHOLD_MM. The distance is host-settable
    // (Vl53l1x::set_distance_mm) and driven live from the LabWired UI input
    // bridge, so PROXIMITY flips NEAR/FAR as you move the slider.
    loop {
        let status = vl_read_register(REG_RESULT_RANGE_STATUS);
        let raw = vl_read_register16(REG_RESULT_RANGE_MM);
        // Same back-conversion the Pololu driver applies in read().
        let range_mm = (((raw as u32) * 2011 + 0x0400) / 0x0800) as u16;

        uart_str("STATUS=");
        uart_u16(status as u16);
        uart_str(" RANGE=");
        uart_u16(range_mm);
        uart_str(" mm PROXIMITY=");
        if range_mm <= PROXIMITY_THRESHOLD_MM {
            uart_str("NEAR\n");
        } else {
            uart_str("FAR\n");
        }

        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
