#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Seeed XIAO ESP32-C6 UART0 + BME280 I2C0 smoke.
//!
//! The firmware uses the C6 I2C0 command-list controller and GPIO matrix to
//! read the Adafruit breakout's chip ID and raw temperature registers. The
//! modeled board route is D4/SDA=GPIO22 and D5/SCL=GPIO23.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

const PCR_I2C0_CONF: *mut u32 = 0x6009_6020 as *mut u32;
const PCR_I2C0_SCLK_CONF: *mut u32 = 0x6009_6024 as *mut u32;

const I2C0: usize = 0x6000_4000;
const GPIO: usize = 0x6009_1000;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO + 0x24) as *mut u32;
const GPIO_OUT_SEL: usize = GPIO + 0x554;
const GPIO_IN_SEL: usize = GPIO + 0x154;
const SDA_PIN: u32 = 22;
const SCL_PIN: u32 = 23;
const I2CEXT0_SCL: u32 = 45;
const I2CEXT0_SDA: u32 = 46;
const GPIO_MATRIX_INPUT_SELECT: u32 = 1 << 7;

const I2C_CTR: *mut u32 = (I2C0 + 0x04) as *mut u32;
const I2C_FIFO_CONF: *mut u32 = (I2C0 + 0x18) as *mut u32;
const I2C_DATA: *mut u32 = (I2C0 + 0x1C) as *mut u32;
const I2C_INT_RAW: *const u32 = (I2C0 + 0x20) as *const u32;
const I2C_INT_CLR: *mut u32 = (I2C0 + 0x24) as *mut u32;

const ADDR: u8 = 0x77;
const OP_RSTART: u32 = 6;
const OP_WRITE: u32 = 1;
const OP_READ: u32 = 3;
const OP_STOP: u32 = 2;
const INT_END_DETECT: u32 = 1 << 3;
const INT_TRANS_COMPLETE: u32 = 1 << 7;
const INT_NACK: u32 = 1 << 10;

#[inline(always)]
unsafe fn reg_write(offset: usize, value: u32) {
    core::ptr::write_volatile((I2C0 + offset) as *mut u32, value);
}

#[inline(always)]
unsafe fn command(index: usize, opcode: u32, bytes: u32) {
    reg_write(0x58 + index * 4, (opcode << 11) | (bytes & 0xff));
}

fn uart_byte(byte: u8) {
    unsafe { core::ptr::write_volatile(UART0_FIFO, u32::from(byte)) }
}

fn uart_str(text: &str) {
    for byte in text.bytes() {
        uart_byte(byte);
    }
}

fn uart_hex_byte(value: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    uart_byte(HEX[(value >> 4) as usize]);
    uart_byte(HEX[(value & 0x0f) as usize]);
}

fn uart_hex20(value: u32) {
    uart_hex_byte(((value >> 16) & 0xff) as u8);
    uart_hex_byte(((value >> 8) & 0xff) as u8);
    uart_hex_byte((value & 0xff) as u8);
}

/// Read one register byte with a genuine repeated-start I2C transaction.
unsafe fn read_register(register: u8) -> Result<u8, ()> {
    // The shared C3/C6 I2C model self-clears the RX/TX FIFO reset bits.
    core::ptr::write_volatile(I2C_FIFO_CONF, (1 << 12) | (1 << 13));
    core::ptr::write_volatile(I2C_INT_CLR, u32::MAX);

    command(0, OP_RSTART, 0);
    command(1, OP_WRITE, 2); // address + W, register address
    command(2, OP_RSTART, 0);
    command(3, OP_WRITE, 1); // address + R
    command(4, OP_READ, 1);
    command(5, OP_STOP, 0);

    core::ptr::write_volatile(I2C_DATA, u32::from(ADDR << 1));
    core::ptr::write_volatile(I2C_DATA, u32::from(register));
    core::ptr::write_volatile(I2C_DATA, u32::from((ADDR << 1) | 1));
    core::ptr::write_volatile(I2C_CTR, 1 << 5); // TRANS_START

    for _ in 0..200_000 {
        let status = core::ptr::read_volatile(I2C_INT_RAW);
        if status & INT_NACK != 0 {
            return Err(());
        }
        if status & (INT_TRANS_COMPLETE | INT_END_DETECT) != 0 {
            return Ok((core::ptr::read_volatile(I2C_DATA) & 0xff) as u8);
        }
    }
    Err(())
}

unsafe fn setup_uart() {
    core::ptr::write_volatile(PCR_UART0_CONF, 1);
    let clock = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
    core::ptr::write_volatile(PCR_UART0_SCLK_CONF, clock | (1 << 22));
    core::ptr::write_volatile(UART0_CLKDIV, 694);
}

unsafe fn setup_i2c() {
    core::ptr::write_volatile(PCR_I2C0_CONF, 1);
    let clock = core::ptr::read_volatile(PCR_I2C0_SCLK_CONF);
    core::ptr::write_volatile(PCR_I2C0_SCLK_CONF, clock | (1 << 22));

    core::ptr::write_volatile(GPIO_ENABLE_W1TS, (1 << SDA_PIN) | (1 << SCL_PIN));
    core::ptr::write_volatile(
        (GPIO_OUT_SEL + SDA_PIN as usize * 4) as *mut u32,
        I2CEXT0_SDA,
    );
    core::ptr::write_volatile(
        (GPIO_OUT_SEL + SCL_PIN as usize * 4) as *mut u32,
        I2CEXT0_SCL,
    );
    core::ptr::write_volatile(
        (GPIO_IN_SEL + I2CEXT0_SDA as usize * 4) as *mut u32,
        GPIO_MATRIX_INPUT_SELECT | SDA_PIN,
    );
    core::ptr::write_volatile(
        (GPIO_IN_SEL + I2CEXT0_SCL as usize * 4) as *mut u32,
        GPIO_MATRIX_INPUT_SELECT | SCL_PIN,
    );

    // 100 kHz style timing from the C6 I2C waveform fixture.
    core::ptr::write_volatile(I2C0 as *mut u32, 199);
    reg_write(0x38, 180 | (19 << 9));
    reg_write(0x30, 29);
    reg_write(0x40, 199);
    reg_write(0x44, 199);
    reg_write(0x4c, 199);
    reg_write(0x48, 199);
}

#[entry]
fn main() -> ! {
    unsafe {
        setup_uart();
        setup_i2c();

        match read_register(0xD0) {
            Ok(0x60) => uart_str("BME280_ID=0x60\n"),
            Ok(id) => {
                uart_str("BME280_ID=0x");
                uart_hex_byte(id);
                uart_str("\nBME280_ID_ERROR\n");
                loop {}
            }
            Err(()) => {
                uart_str("BME280_I2C_ERROR\n");
                loop {}
            }
        }

        match read_register(0xFA) {
            Ok(msb) => match read_register(0xFB) {
                Ok(lsb) => match read_register(0xFC) {
                    Ok(xlsb) => {
                        let raw =
                            (u32::from(msb) << 12) | (u32::from(lsb) << 4) | (u32::from(xlsb) >> 4);
                        uart_str("BME280_TEMP_RAW=0x");
                        uart_hex20(raw);
                        uart_str("\n");
                    }
                    Err(()) => uart_str("BME280_I2C_ERROR\n"),
                },
                Err(()) => uart_str("BME280_I2C_ERROR\n"),
            },
            Err(()) => uart_str("BME280_I2C_ERROR\n"),
        }
    }
    loop {}
}
