#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Exercise the ESP32-C3-DevKit-RUST-2's onboard WS2812B and SHTC3.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_OUT_W1TS: *mut u32 = 0x6000_4008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6000_400C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6000_4024 as *mut u32;
const IO_MUX_GPIO2: *mut u32 = 0x6000_900C as *mut u32;
const GPIO2_MASK: u32 = 1 << 2;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const GPIO: usize = 0x6000_4000;
const GPIO_OUT_SEL: usize = GPIO + 0x554;
const GPIO_IN_SEL: usize = GPIO + 0x154;
const I2C0: usize = 0x6001_3000;
const I2C_FIFO_CONF: *mut u32 = (I2C0 + 0x18) as *mut u32;
const I2C_DATA: *mut u32 = (I2C0 + 0x1C) as *mut u32;
const I2C_INT_RAW: *const u32 = (I2C0 + 0x20) as *const u32;
const I2C_INT_CLR: *mut u32 = (I2C0 + 0x24) as *mut u32;
const I2C_CTR: *mut u32 = (I2C0 + 0x04) as *mut u32;
const I2C_CMD0: usize = I2C0 + 0x58;
const SDA_PIN: u32 = 10;
const SCL_PIN: u32 = 8;
const I2C0_SCL_SIGNAL: u32 = 53;
const I2C0_SDA_SIGNAL: u32 = 54;
const GPIO_MATRIX_INPUT_SELECT: u32 = 1 << 6;
const SHTC3_ADDR: u8 = 0x70;
const OP_RSTART: u32 = 6;
const OP_WRITE: u32 = 1;
const OP_READ: u32 = 3;
const OP_STOP: u32 = 2;
const INT_END_DETECT: u32 = 1 << 3;
const INT_TRANS_COMPLETE: u32 = 1 << 7;
const INT_NACK: u32 = 1 << 10;

// At 160 MHz the model treats high pulses below 500 ns as a 0 and longer
// pulses as a 1. Keep the WS2812 bit cell near 1.25 us.
#[inline(always)]
fn short_high() {
    unsafe { core::arch::asm!(".rept 40\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn long_high() {
    unsafe { core::arch::asm!(".rept 96\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn short_low() {
    unsafe { core::arch::asm!(".rept 150\n nop\n .endr", options(nomem, nostack)) }
}

#[inline(always)]
fn long_low() {
    unsafe { core::arch::asm!(".rept 94\n nop\n .endr", options(nomem, nostack)) }
}

fn uart_str(text: &str) {
    for byte in text.bytes() {
        unsafe { core::ptr::write_volatile(UART0_FIFO, u32::from(byte)) }
    }
}

#[inline(always)]
unsafe fn reg_write(offset: usize, value: u32) {
    core::ptr::write_volatile((I2C0 + offset) as *mut u32, value)
}

#[inline(always)]
unsafe fn command(index: usize, opcode: u32, bytes: u32) {
    core::ptr::write_volatile(
        (I2C_CMD0 + index * 4) as *mut u32,
        (opcode << 11) | (bytes & 0xff),
    );
}

/// Wait for the SHTC3 low-power conversion bound (0.8 ms max) plus margin.
/// Two RISC-V instructions per iteration at 160 MHz make 80,000 iterations
/// slightly longer than 1 ms in the model and on the board.
fn measurement_delay() {
    let remaining = 80_000u32;
    unsafe {
        core::arch::asm!(
            "2:",
            "addi {remaining}, {remaining}, -1",
            "bnez {remaining}, 2b",
            remaining = inout(reg) remaining => _,
            options(nomem, nostack)
        );
    }
}

fn wakeup_delay() {
    let remaining = 24_000u32;
    unsafe {
        core::arch::asm!(
            "2:",
            "addi {remaining}, {remaining}, -1",
            "bnez {remaining}, 2b",
            remaining = inout(reg) remaining => _,
            options(nomem, nostack)
        );
    }
}

unsafe fn setup_i2c() {
    let route_pads = (1 << SDA_PIN) | (1 << SCL_PIN);
    core::ptr::write_volatile(GPIO_ENABLE_W1TS, route_pads);
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

    // Standard-mode I2C at 100 kHz from the C3's 40 MHz crystal source.
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

unsafe fn wait_i2c() -> Result<(), ()> {
    for _ in 0..200_000 {
        let status = core::ptr::read_volatile(I2C_INT_RAW);
        if status & INT_NACK != 0 {
            return Err(());
        }
        if status & (INT_TRANS_COMPLETE | INT_END_DETECT) != 0 {
            return Ok(());
        }
    }
    Err(())
}

/// Send one 16-bit SHTC3 command in a standalone I2C transaction.
unsafe fn write_shtc3_command(code: u16) -> Result<(), ()> {
    core::ptr::write_volatile(I2C_FIFO_CONF, (1 << 12) | (1 << 13));
    core::ptr::write_volatile(I2C_INT_CLR, u32::MAX);
    command(0, OP_RSTART, 0);
    command(1, OP_WRITE, 3); // address + the two command bytes
    command(2, OP_STOP, 0);
    core::ptr::write_volatile(I2C_DATA, u32::from(SHTC3_ADDR << 1));
    core::ptr::write_volatile(I2C_DATA, u32::from((code >> 8) as u8));
    core::ptr::write_volatile(I2C_DATA, u32::from(code as u8));
    core::ptr::write_volatile(I2C_CTR, 1 << 5); // TRANS_START
    wait_i2c()
}

/// Read the six-byte SHTC3 T/RH response, including both CRC bytes.
unsafe fn read_shtc3_measurement() -> Result<[u8; 6], ()> {
    core::ptr::write_volatile(I2C_FIFO_CONF, (1 << 12) | (1 << 13));
    core::ptr::write_volatile(I2C_INT_CLR, u32::MAX);
    command(0, OP_RSTART, 0);
    command(1, OP_WRITE, 1); // address + R
    command(2, OP_READ, 6);
    command(3, OP_STOP, 0);
    core::ptr::write_volatile(I2C_DATA, u32::from((SHTC3_ADDR << 1) | 1));
    core::ptr::write_volatile(I2C_CTR, 1 << 5); // TRANS_START
    wait_i2c()?;

    let mut response = [0u8; 6];
    for byte in &mut response {
        *byte = (core::ptr::read_volatile(I2C_DATA) & 0xff) as u8;
    }
    Ok(response)
}

fn sensirion_crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0xFFu8;
    for byte in bytes {
        crc ^= *byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn check_shtc3() -> bool {
    unsafe {
        setup_i2c();
        if write_shtc3_command(0x3517).is_err() {
            return false;
        }
        wakeup_delay();
        if write_shtc3_command(0x609C).is_err() {
            return false;
        }
        measurement_delay();
        let Ok(response) = read_shtc3_measurement() else {
            return false;
        };
        let temperature_raw = u16::from_be_bytes([response[0], response[1]]);
        let humidity_raw = u16::from_be_bytes([response[3], response[4]]);
        response[2] == sensirion_crc8(&response[0..2])
            && response[5] == sensirion_crc8(&response[3..5])
            && temperature_raw == 0x64F0
            && humidity_raw == 0x6666
    }
}

#[inline(always)]
unsafe fn send_ws2812_byte(byte: u8) {
    for bit in (0..8).rev() {
        let one = (byte & (1 << bit)) != 0;
        core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO2_MASK);
        if one {
            long_high();
        } else {
            short_high();
        }
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO2_MASK);
        if one {
            long_low();
        } else {
            short_low();
        }
    }
}

#[inline(never)]
fn ws2812_reset_gap() {
    for _ in 0..8_000 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // GPIO2's IO_MUX reset value selects its GPIO function; make that
        // selection explicit before enabling its output driver.
        let gpio2_mux = core::ptr::read_volatile(IO_MUX_GPIO2);
        core::ptr::write_volatile(IO_MUX_GPIO2, gpio2_mux & !(0b111 << 12));
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO2_MASK);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO2_MASK);

        // The board's RGB LED is a WS2812B. 00 FF 00 is red in GRB order.
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();

        if check_shtc3() {
            uart_str("ESP32_C3_RUST2_SHTC3_OK_T24_RH40\n");
        } else {
            uart_str("ESP32_C3_RUST2_SHTC3_ERROR\n");
        }
    }

    loop {}
}
