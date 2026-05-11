// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// STM32F407 I²C onboarding firmware.
//
// Actually drives the legacy F1/F2/F4 I²C state machine — START → ADDR
// → DR transfers with explicit SR1/SR2 polling — against two devices
// on I²C1:
//
//   - AHT20  @ 0x38 — command-stream protocol with BUSY-poll
//   - BMP280 @ 0x76 — register-bank chip-ID read (expect 0x58)
//
// The same ELF runs against silicon and against the simulator. When
// hardware lands and the oracle capture is recorded, this is the
// reference firmware that produces the truth trace.
//
// LED on PA5 toggles each successful poll cycle so a logic-analyser /
// simulator-side board_io observer can see end-to-end progress.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Base addresses (STM32F407, matches configs/chips/stm32f407.yaml) ──
const RCC_BASE: u32 = 0x40023800;
const GPIOA_BASE: u32 = 0x40020000;
const GPIOB_BASE: u32 = 0x40020400;
const I2C1_BASE: u32 = 0x40005400;

// ── RCC registers ─────────────────────────────────────────────────────
const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x40) as *mut u32;
const RCC_AHB1ENR_GPIOAEN: u32 = 1 << 0;
const RCC_AHB1ENR_GPIOBEN: u32 = 1 << 1;
const RCC_APB1ENR_I2C1EN: u32 = 1 << 21;

// ── GPIO registers (stm32v2 layout) ───────────────────────────────────
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_ODR: *mut u32 = (GPIOA_BASE + 0x14) as *mut u32;

const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_OTYPER: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;
const GPIOB_OSPEEDR: *mut u32 = (GPIOB_BASE + 0x08) as *mut u32;
const GPIOB_PUPDR: *mut u32 = (GPIOB_BASE + 0x0C) as *mut u32;
const GPIOB_AFRL: *mut u32 = (GPIOB_BASE + 0x20) as *mut u32;

// ── I²C registers (stm32f1 legacy layout, shared with F4) ─────────────
const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *mut u32 = (I2C1_BASE + 0x14) as *mut u32;
const I2C1_SR2: *mut u32 = (I2C1_BASE + 0x18) as *mut u32;
const I2C1_CCR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_TRISE: *mut u32 = (I2C1_BASE + 0x20) as *mut u32;

// CR1 bits
const I2C_CR1_PE: u32 = 1 << 0;
const I2C_CR1_START: u32 = 1 << 8;
const I2C_CR1_STOP: u32 = 1 << 9;
const I2C_CR1_ACK: u32 = 1 << 10;
const I2C_CR1_SWRST: u32 = 1 << 15;

// SR1 bits
const I2C_SR1_SB: u32 = 1 << 0;
const I2C_SR1_ADDR: u32 = 1 << 1;
const I2C_SR1_BTF: u32 = 1 << 2;
const I2C_SR1_RXNE: u32 = 1 << 6;
const I2C_SR1_TXE: u32 = 1 << 7;

// Device addresses (7-bit, shifted into the address byte below)
const AHT20_ADDR: u8 = 0x38;
const BMP280_ADDR: u8 = 0x76;

// AHT20 status BUSY bit
const AHT20_STATUS_BUSY: u8 = 0x80;

// Generous polling budget — bus is fully synchronous in the simulator,
// and real silicon at 100 kHz needs only tens of cycles per byte.
const POLL_BUDGET: u32 = 100_000;

#[entry]
fn main() -> ! {
    unsafe {
        // Encode progress through PA pins as the firmware advances. The
        // simulator-side e2e test polls GPIOA_ODR every step and records
        // the max value seen, so partial progress shows up even if the
        // firmware never reaches the "success" write.
        //   PA0 — clock_init done
        //   PA1 — gpio_init done
        //   PA2 — i2c1_init done
        //   PA3 — AHT20 trigger phase done
        //   PA4 — AHT20 busy clear seen
        //   PA5 — full success (both AHT20 + BMP280 read OK)
        //   PA6 — AHT20 full read returned OK
        //   PA7 — BMP280 chip-ID returned 0x58
        clock_init();
        write_volatile(GPIOA_ODR, 1 << 0);
        gpio_init();
        write_volatile(GPIOA_ODR, (1 << 0) | (1 << 1));
        i2c1_init();
        write_volatile(GPIOA_ODR, (1 << 0) | (1 << 1) | (1 << 2));

        let aht20_ok = aht20_one_shot();
        let bmp280_ok = bmp280_chip_id() == 0x58;

        let mut bits = (1u32 << 0) | (1 << 1) | (1 << 2);
        if aht20_ok {
            bits |= 1 << 6;
        }
        if bmp280_ok {
            bits |= 1 << 7;
        }
        if aht20_ok && bmp280_ok {
            bits |= 1 << 5;
        }
        write_volatile(GPIOA_ODR, bits);

        loop {
            cortex_m::asm::nop();
        }
    }
}

// ── Init helpers ──────────────────────────────────────────────────────

unsafe fn clock_init() {
    let mut v = read_volatile(RCC_AHB1ENR);
    v |= RCC_AHB1ENR_GPIOAEN | RCC_AHB1ENR_GPIOBEN;
    write_volatile(RCC_AHB1ENR, v);

    let mut v = read_volatile(RCC_APB1ENR);
    v |= RCC_APB1ENR_I2C1EN;
    write_volatile(RCC_APB1ENR, v);
}

unsafe fn gpio_init() {
    // PA5 = output (LED). MODER: 2 bits per pin, "01" = output.
    let mut moder = read_volatile(GPIOA_MODER);
    moder &= !(0b11 << (5 * 2));
    moder |= 0b01 << (5 * 2);
    write_volatile(GPIOA_MODER, moder);

    // PB6 (SCL) + PB7 (SDA) = alternate function (10), open-drain,
    // high speed, pull-up, AF4 (I2C1).
    let mut moder = read_volatile(GPIOB_MODER);
    moder &= !(0b11 << (6 * 2));
    moder &= !(0b11 << (7 * 2));
    moder |= 0b10 << (6 * 2);
    moder |= 0b10 << (7 * 2);
    write_volatile(GPIOB_MODER, moder);

    let mut otyper = read_volatile(GPIOB_OTYPER);
    otyper |= (1 << 6) | (1 << 7);
    write_volatile(GPIOB_OTYPER, otyper);

    let mut ospeedr = read_volatile(GPIOB_OSPEEDR);
    ospeedr |= 0b11 << (6 * 2);
    ospeedr |= 0b11 << (7 * 2);
    write_volatile(GPIOB_OSPEEDR, ospeedr);

    let mut pupdr = read_volatile(GPIOB_PUPDR);
    pupdr &= !(0b11 << (6 * 2));
    pupdr &= !(0b11 << (7 * 2));
    pupdr |= 0b01 << (6 * 2);
    pupdr |= 0b01 << (7 * 2);
    write_volatile(GPIOB_PUPDR, pupdr);

    // AF4 = I²C1. AFRL bits [27:24] for PB6, [31:28] for PB7.
    let mut afrl = read_volatile(GPIOB_AFRL);
    afrl &= !(0xF << (6 * 4));
    afrl &= !(0xF << (7 * 4));
    afrl |= 0x4 << (6 * 4);
    afrl |= 0x4 << (7 * 4);
    write_volatile(GPIOB_AFRL, afrl);
}

unsafe fn i2c1_init() {
    // Software reset to start from a known state.
    write_volatile(I2C1_CR1, I2C_CR1_SWRST);
    write_volatile(I2C1_CR1, 0);

    // Peripheral clock frequency (CR2.FREQ): APB1 = 16 MHz (default boot,
    // HSI). Real silicon with PLL will need this updated post-clock-tree
    // configuration; for the simulator the value is informational since
    // the I²C model doesn't gate on it.
    write_volatile(I2C1_CR2, 16);

    // CCR for 100 kHz standard mode: PCLK1 / (2 * 100kHz) = 16M/200k = 80.
    write_volatile(I2C1_CCR, 80);

    // TRISE for standard mode: PCLK1_MHz + 1 = 17.
    write_volatile(I2C1_TRISE, 17);

    // Enable the peripheral.
    write_volatile(I2C1_CR1, I2C_CR1_PE);
}

// ── I²C primitives — actually drive the state machine ────────────────

unsafe fn i2c_start() -> bool {
    let cr1 = read_volatile(I2C1_CR1);
    write_volatile(I2C1_CR1, cr1 | I2C_CR1_START);
    poll_sr1(I2C_SR1_SB)
}

unsafe fn i2c_send_address(addr7: u8, read: bool) -> bool {
    let byte = (addr7 << 1) | (read as u8);
    write_volatile(I2C1_DR, byte as u32);
    if !poll_sr1(I2C_SR1_ADDR) {
        return false;
    }
    // ADDR is cleared by reading SR1 then SR2 (the read above already
    // touched SR1 inside poll_sr1, so finish with an SR2 read).
    let _ = read_volatile(I2C1_SR2);
    true
}

unsafe fn i2c_write_byte(b: u8) -> bool {
    if !poll_sr1(I2C_SR1_TXE) {
        return false;
    }
    write_volatile(I2C1_DR, b as u32);
    poll_sr1(I2C_SR1_BTF)
}

unsafe fn i2c_read_byte_ack() -> Option<u8> {
    let cr1 = read_volatile(I2C1_CR1);
    write_volatile(I2C1_CR1, cr1 | I2C_CR1_ACK);
    if !poll_sr1(I2C_SR1_RXNE) {
        return None;
    }
    Some(read_volatile(I2C1_DR) as u8)
}

unsafe fn i2c_read_byte_nack() -> Option<u8> {
    let cr1 = read_volatile(I2C1_CR1);
    write_volatile(I2C1_CR1, (cr1 & !I2C_CR1_ACK) | I2C_CR1_STOP);
    if !poll_sr1(I2C_SR1_RXNE) {
        return None;
    }
    Some(read_volatile(I2C1_DR) as u8)
}

unsafe fn i2c_stop() {
    let cr1 = read_volatile(I2C1_CR1);
    write_volatile(I2C1_CR1, cr1 | I2C_CR1_STOP);
}

unsafe fn poll_sr1(mask: u32) -> bool {
    for _ in 0..POLL_BUDGET {
        if (read_volatile(I2C1_SR1) & mask) != 0 {
            return true;
        }
    }
    false
}

// ── Device helpers ────────────────────────────────────────────────────

/// Trigger an AHT20 measurement, poll BUSY until clear, read 7 bytes.
/// Returns true if the BUSY bit clears within the polling budget.
unsafe fn aht20_one_shot() -> bool {
    // Trigger measurement: 0xAC 0x33 0x00
    if !i2c_start() {
        return false;
    }
    if !i2c_send_address(AHT20_ADDR, false) {
        return false;
    }
    if !i2c_write_byte(0xAC) || !i2c_write_byte(0x33) || !i2c_write_byte(0x00) {
        return false;
    }
    i2c_stop();

    // Poll status until BUSY clears.
    let mut busy_cleared = false;
    for _ in 0..16 {
        if !i2c_start() {
            return false;
        }
        if !i2c_send_address(AHT20_ADDR, true) {
            return false;
        }
        let status = match i2c_read_byte_nack() {
            Some(b) => b,
            None => return false,
        };
        if (status & AHT20_STATUS_BUSY) == 0 {
            busy_cleared = true;
            break;
        }
    }
    if !busy_cleared {
        return false;
    }

    // Read all 7 bytes in one transaction.
    if !i2c_start() {
        return false;
    }
    if !i2c_send_address(AHT20_ADDR, true) {
        return false;
    }
    let mut buf = [0u8; 7];
    for byte in buf.iter_mut().take(6) {
        match i2c_read_byte_ack() {
            Some(b) => *byte = b,
            None => return false,
        }
    }
    buf[6] = match i2c_read_byte_nack() {
        Some(b) => b,
        None => return false,
    };

    // Caller could verify CRC8 here; for now we trust the bus round-trip
    // and use BUSY-clear as the success signal. Hardware oracle capture
    // will diff the full 7-byte payload.
    let _ = buf;
    true
}

/// Read the BMP280 chip-ID register (0xD0) and return its value.
unsafe fn bmp280_chip_id() -> u8 {
    if !i2c_start() {
        return 0;
    }
    if !i2c_send_address(BMP280_ADDR, false) {
        return 0;
    }
    if !i2c_write_byte(0xD0) {
        return 0;
    }
    if !i2c_start() {
        return 0;
    }
    if !i2c_send_address(BMP280_ADDR, true) {
        return 0;
    }
    i2c_read_byte_nack().unwrap_or(0)
}
