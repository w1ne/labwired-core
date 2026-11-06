#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Read real BME280 registers over an ESP32-C3 I2C0 path.
//!
//! The default profile routes GPIO4/GPIO5 on the DevKitM-1-N4X; `--features
//! xiao` routes the XIAO's standard D4/D5 header at GPIO6/GPIO7. Firmware
//! configures the GPIO matrix, runs an address-write/repeated-start transaction,
//! and prints the returned chip ID and temperature ADC sample on UART0.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

const I2C0: usize = 0x6001_3000;
const GPIO: usize = 0x6000_4000;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO + 0x24) as *mut u32;
const GPIO_OUT_SEL: usize = GPIO + 0x554;
const GPIO_IN_SEL: usize = GPIO + 0x154;

#[cfg(feature = "xiao")]
const SDA_PIN: u32 = 6;
#[cfg(not(feature = "xiao"))]
const SDA_PIN: u32 = 4;
#[cfg(feature = "xiao")]
const SCL_PIN: u32 = 7;
#[cfg(not(feature = "xiao"))]
const SCL_PIN: u32 = 5;
const I2C0_SCL_SIGNAL: u32 = 53;
const I2C0_SDA_SIGNAL: u32 = 54;
const GPIO_MATRIX_INPUT_SELECT: u32 = 1 << 6;

const I2C_CTR: *mut u32 = (I2C0 + 0x04) as *mut u32;
const I2C_FIFO_CONF: *mut u32 = (I2C0 + 0x18) as *mut u32;
const I2C_DATA: *mut u32 = (I2C0 + 0x1C) as *mut u32;
const I2C_INT_RAW: *const u32 = (I2C0 + 0x20) as *const u32;
const I2C_INT_CLR: *mut u32 = (I2C0 + 0x24) as *mut u32;
const I2C_CMD0: usize = I2C0 + 0x58;

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
    core::ptr::write_volatile((I2C0 + offset) as *mut u32, value)
}

#[inline(always)]
unsafe fn command(index: usize, opcode: u32, bytes: u32) {
    core::ptr::write_volatile(
        (I2C_CMD0 + index * 4) as *mut u32,
        (opcode << 11) | (bytes & 0xff),
    )
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

/// Read one BME280 register using a write-pointer/repeated-start transaction.
unsafe fn read_register(register: u8) -> Result<u8, ()> {
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

unsafe fn setup_i2c() {
    core::ptr::write_volatile(GPIO_ENABLE_W1TS, (1 << SDA_PIN) | (1 << SCL_PIN));
    core::ptr::write_volatile(
        (GPIO_OUT_SEL + SDA_PIN as usize * 4) as *mut u32,
        I2C0_SDA_SIGNAL,
    );
    core::ptr::write_volatile(
        (GPIO_OUT_SEL + SCL_PIN as usize * 4) as *mut u32,
        I2C0_SCL_SIGNAL,
    );
    core::ptr::write_volatile(
        (GPIO_IN_SEL + I2C0_SDA_SIGNAL as usize * 4) as *mut u32,
        GPIO_MATRIX_INPUT_SELECT | SDA_PIN,
    );
    core::ptr::write_volatile(
        (GPIO_IN_SEL + I2C0_SCL_SIGNAL as usize * 4) as *mut u32,
        GPIO_MATRIX_INPUT_SELECT | SCL_PIN,
    );

    // 100 kHz on the C3's 40 MHz crystal source: 200 low + 200 high ticks.
    reg_write(0x00, 199); // SCL_LOW_PERIOD
    reg_write(0x30, 20); // SDA_HOLD
    reg_write(0x34, 20); // SDA_SAMPLE
    reg_write(0x38, 199); // SCL_HIGH_PERIOD
    reg_write(0x40, 20); // SCL_START_HOLD
    reg_write(0x44, 20); // SCL_RSTART_SETUP
    reg_write(0x48, 20); // SCL_STOP_HOLD
    reg_write(0x4c, 20); // SCL_STOP_SETUP
    reg_write(0x54, 0); // divide-by-one XTAL source
}

#[entry]
fn main() -> ! {
    unsafe {
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
