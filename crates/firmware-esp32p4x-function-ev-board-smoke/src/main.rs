#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! BME280 ID gate plus GPIO8 output smoke for the ESP32-P4X Function EV Board.
//!
//! The modeled BME280 is attached directly to I2C0 at the controller boundary.
//! GPIO-matrix routing and a physical SDA/SCL waveform are not part of this
//! focused profile. The image is loaded at its RISC-V entry point; it does not
//! exercise the mask ROM or SPI-flash boot path.

use panic_halt as _;
use riscv_rt::entry;

const I2C0: usize = 0x500C_4000;
const GPIO_BASE: usize = 0x500E_0000;

const I2C_CTR: *mut u32 = (I2C0 + 0x04) as *mut u32;
const I2C_FIFO_CONF: *mut u32 = (I2C0 + 0x18) as *mut u32;
const I2C_DATA: *mut u32 = (I2C0 + 0x1C) as *mut u32;
const I2C_INT_RAW: *const u32 = (I2C0 + 0x20) as *const u32;
const I2C_INT_CLR: *mut u32 = (I2C0 + 0x24) as *mut u32;
const I2C_CMD0: usize = I2C0 + 0x58;

const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO_FUNC8_OUT_SEL_CFG: *mut u32 = (GPIO_BASE + 0x578) as *mut u32;
const IO_MUX_GPIO8: *mut u32 = 0x500E_1024 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;

const BME280_ADDRESS: u8 = 0x77;
const BME280_ID_REGISTER: u8 = 0xD0;
const OP_RSTART: u32 = 6;
const OP_WRITE: u32 = 1;
const OP_READ: u32 = 3;
const OP_STOP: u32 = 2;
const INT_END_DETECT: u32 = 1 << 3;
const INT_TRANS_COMPLETE: u32 = 1 << 7;
const INT_NACK: u32 = 1 << 10;

#[inline(always)]
unsafe fn command(index: usize, opcode: u32, bytes: u32) {
    core::ptr::write_volatile(
        (I2C_CMD0 + index * 4) as *mut u32,
        (opcode << 11) | (bytes & 0xff),
    );
}

/// Read BME280 register 0xD0 using an address-write/repeated-start read.
unsafe fn read_bme280_id() -> Option<u8> {
    core::ptr::write_volatile(I2C_FIFO_CONF, (1 << 12) | (1 << 13));
    core::ptr::write_volatile(I2C_INT_CLR, u32::MAX);

    command(0, OP_RSTART, 0);
    command(1, OP_WRITE, 2); // address + W, register address
    command(2, OP_RSTART, 0);
    command(3, OP_WRITE, 1); // address + R
    command(4, OP_READ, 1);
    command(5, OP_STOP, 0);

    core::ptr::write_volatile(I2C_DATA, u32::from(BME280_ADDRESS << 1));
    core::ptr::write_volatile(I2C_DATA, u32::from(BME280_ID_REGISTER));
    core::ptr::write_volatile(I2C_DATA, u32::from((BME280_ADDRESS << 1) | 1));
    core::ptr::write_volatile(I2C_CTR, 1 << 5); // TRANS_START

    for _ in 0..200_000 {
        let status = core::ptr::read_volatile(I2C_INT_RAW);
        if status & INT_NACK != 0 {
            return None;
        }
        if status & (INT_TRANS_COMPLETE | INT_END_DETECT) != 0 {
            return Some((core::ptr::read_volatile(I2C_DATA) & 0xff) as u8);
        }
    }
    None
}

#[inline(always)]
unsafe fn gpio8_pulse() {
    core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO8_MASK);
    core::arch::asm!("nop", options(nomem, nostack));
    core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
    core::arch::asm!("nop", options(nomem, nostack));
}

#[entry]
fn main() -> ! {
    unsafe {
        // GPIO8 function select is 1. Keep drive strength at its default 2.
        core::ptr::write_volatile(IO_MUX_GPIO8, (2 << 10) | (1 << 12));
        // Select GPIO_OUT and the output-enable path for GPIO8.
        core::ptr::write_volatile(GPIO_FUNC8_OUT_SEL_CFG, 256 | (1 << 10));
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);

        // The GPIO smoke passes only if the sensor returned its documented ID.
        if read_bme280_id() == Some(0x60) {
            // Emit 48 transitions (24 high/low pulses), then leave a compact
            // stable trace for the committed local simulator evidence.
            for _ in 0..24 {
                gpio8_pulse();
            }
        }
    }

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
