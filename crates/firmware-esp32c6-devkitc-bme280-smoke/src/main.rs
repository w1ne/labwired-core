#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Read the BME280 ID and a raw-temperature sample on ESP32-C6 I2C0.
//!
//! The board smoke uses the 8 MiB flash layout of the ESP32-C6-DevKitC-1-N8.
//! I2C0 is matrix-routed to GPIO6 (SDA) and GPIO7 (SCL); UART0 writes are
//! captured as the board's USB-to-UART console.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
const PCR_I2C0_CONF: *mut u32 = 0x6009_6020 as *mut u32;
const IO_MUX_GPIO16: *mut u32 = 0x6009_0044 as *mut u32;

const GPIO_BASE: u32 = 0x6009_1000;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO_OUT_SEL_SDA: *mut u32 = (GPIO_BASE + 0x554 + 6 * 4) as *mut u32;
const GPIO_OUT_SEL_SCL: *mut u32 = (GPIO_BASE + 0x554 + 7 * 4) as *mut u32;
const GPIO_IN_SEL_SDA: *mut u32 = (GPIO_BASE + 0x154 + 46 * 4) as *mut u32;
const GPIO_IN_SEL_SCL: *mut u32 = (GPIO_BASE + 0x154 + 45 * 4) as *mut u32;

const I2C_BASE: u32 = 0x6000_4000;
const I2C_CTR: *mut u32 = (I2C_BASE + 0x04) as *mut u32;
const I2C_FIFO_CONF: *mut u32 = (I2C_BASE + 0x18) as *mut u32;
const I2C_DATA: *mut u32 = (I2C_BASE + 0x1C) as *mut u32;
const I2C_INT_RAW: *mut u32 = (I2C_BASE + 0x20) as *mut u32;
const I2C_INT_CLR: *mut u32 = (I2C_BASE + 0x24) as *mut u32;
const I2C_CLK_CONF: *mut u32 = (I2C_BASE + 0x54) as *mut u32;
const I2C_COMD0: u32 = I2C_BASE + 0x58;
const I2C_SCL_LOW: *mut u32 = I2C_BASE as *mut u32;
const I2C_SDA_HOLD: *mut u32 = (I2C_BASE + 0x30) as *mut u32;
const I2C_SDA_SAMPLE: *mut u32 = (I2C_BASE + 0x34) as *mut u32;
const I2C_SCL_HIGH: *mut u32 = (I2C_BASE + 0x38) as *mut u32;
const I2C_START_HOLD: *mut u32 = (I2C_BASE + 0x40) as *mut u32;
const I2C_RSTART_SETUP: *mut u32 = (I2C_BASE + 0x44) as *mut u32;
const I2C_STOP_HOLD: *mut u32 = (I2C_BASE + 0x48) as *mut u32;
const I2C_STOP_SETUP: *mut u32 = (I2C_BASE + 0x4C) as *mut u32;

const MASTER_MODE: u32 = 1 << 4;
const I2C_FORCE_OUT: u32 = 3;
const I2C_CLK_EN: u32 = 1 << 8;
const I2C_CONF_UPGATE: u32 = 1 << 11;
const I2C_TRANS_START: u32 = 1 << 5;
const FIFO_RUN: u32 = (1 << 14) | (8 << 5) | 1;
const FIFO_RESET: u32 = FIFO_RUN | (1 << 13) | (1 << 12);
const INT_TRANS_COMPLETE: u32 = 1 << 7;
const INT_NACK: u32 = 1 << 10;
const OP_RSTART: u32 = 6;
const OP_WRITE: u32 = 1;
const OP_READ: u32 = 3;
const OP_STOP: u32 = 2;
const ACK_CHECK: u32 = 1 << 8;
const NACK_LAST: u32 = 1 << 10;
const BME280_ADDRESS: u32 = 0x77;

#[inline]
unsafe fn write(register: *mut u32, value: u32) {
    core::ptr::write_volatile(register, value);
}

#[inline]
unsafe fn read(register: *mut u32) -> u32 {
    core::ptr::read_volatile(register)
}

fn uart_byte(value: u8) {
    unsafe { write(UART0_FIFO, u32::from(value)) }
}

fn uart_str(text: &[u8]) {
    for &byte in text {
        uart_byte(byte);
    }
}

fn uart_hex_byte(value: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    uart_byte(HEX[(value >> 4) as usize]);
    uart_byte(HEX[(value & 0x0F) as usize]);
}

fn uart_hex20(value: u32) {
    uart_hex_byte((value >> 16) as u8);
    uart_hex_byte((value >> 8) as u8);
    uart_hex_byte(value as u8);
}

unsafe fn command(index: u32, opcode: u32, bytes: u32, flags: u32) {
    let register = (I2C_COMD0 + index * 4) as *mut u32;
    write(register, (opcode << 11) | (bytes & 0xFF) | flags);
}

unsafe fn setup_uart() {
    write(PCR_UART0_CONF, 1);
    write(PCR_UART0_SCLK_CONF, read(PCR_UART0_SCLK_CONF) | (1 << 22));
    write(IO_MUX_GPIO16, 1); // U0TXD on GPIO16, the DevKitC USB-to-UART bridge.
    write(UART0_CLKDIV, 694);
}

unsafe fn setup_i2c() {
    write(PCR_I2C0_CONF, 1); // enable the C6 PCR I2C0 APB clock

    // C6 GPIO matrix: I2CEXT0_SDA=46, I2CEXT0_SCL=45; IN_SEL.SEL is bit 7.
    write(GPIO_OUT_SEL_SDA, 46);
    write(GPIO_OUT_SEL_SCL, 45);
    write(GPIO_IN_SEL_SDA, (1 << 7) | 6);
    write(GPIO_IN_SEL_SCL, (1 << 7) | 7);
    write(GPIO_ENABLE_W1TS, (1 << 6) | (1 << 7));

    write(I2C_CTR, MASTER_MODE | I2C_FORCE_OUT | I2C_CLK_EN);
    write(I2C_CLK_CONF, 1 << 21); // active XTAL source
    write(I2C_SCL_LOW, 199);
    write(I2C_SCL_HIGH, 180 | (19 << 9));
    write(I2C_SDA_HOLD, 29);
    write(I2C_SDA_SAMPLE, 39);
    write(I2C_START_HOLD, 199);
    write(I2C_RSTART_SETUP, 199);
    write(I2C_STOP_HOLD, 199);
    write(I2C_STOP_SETUP, 199);
    write(
        I2C_CTR,
        MASTER_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_CONF_UPGATE,
    );
}

unsafe fn read_register(register: u8) -> Result<u8, ()> {
    write(I2C_FIFO_CONF, FIFO_RESET);
    write(I2C_FIFO_CONF, FIFO_RUN);
    write(I2C_INT_CLR, u32::MAX);
    write(I2C_DATA, BME280_ADDRESS << 1);
    write(I2C_DATA, u32::from(register));
    write(I2C_DATA, (BME280_ADDRESS << 1) | 1);
    command(0, OP_RSTART, 0, 0);
    command(1, OP_WRITE, 2, ACK_CHECK);
    command(2, OP_RSTART, 0, 0);
    command(3, OP_WRITE, 1, ACK_CHECK);
    command(4, OP_READ, 1, NACK_LAST);
    command(5, OP_STOP, 0, 0);
    write(
        I2C_CTR,
        MASTER_MODE | I2C_FORCE_OUT | I2C_CLK_EN | I2C_TRANS_START,
    );

    for _ in 0..500_000 {
        let status = read(I2C_INT_RAW);
        if status & INT_NACK != 0 {
            return Err(());
        }
        if status & INT_TRANS_COMPLETE != 0 {
            return Ok(read(I2C_DATA) as u8);
        }
    }
    Err(())
}

#[entry]
fn main() -> ! {
    unsafe {
        setup_uart();
        setup_i2c();

        match read_register(0xD0) {
            Ok(0x60) => {
                uart_str(b"BME280_ID=0x");
                uart_hex_byte(0x60);
                uart_str(b"\n");
            }
            Ok(id) => {
                uart_str(b"BME280_ID_ERROR=0x");
                uart_hex_byte(id);
                uart_str(b"\n");
                loop {}
            }
            Err(()) => {
                uart_str(b"BME280_I2C_ERROR\n");
                loop {}
            }
        }

        let msb = match read_register(0xFA) {
            Ok(value) => value,
            Err(()) => {
                uart_str(b"BME280_I2C_ERROR\n");
                loop {}
            }
        };
        let lsb = match read_register(0xFB) {
            Ok(value) => value,
            Err(()) => {
                uart_str(b"BME280_I2C_ERROR\n");
                loop {}
            }
        };
        let xlsb = match read_register(0xFC) {
            Ok(value) => value,
            Err(()) => {
                uart_str(b"BME280_I2C_ERROR\n");
                loop {}
            }
        };
        let raw = (u32::from(msb) << 12) | (u32::from(lsb) << 4) | u32::from(xlsb >> 4);
        uart_str(b"BME280_TEMP_RAW=0x");
        uart_hex20(raw);
        uart_str(b"\n");
    }

    loop {}
}
