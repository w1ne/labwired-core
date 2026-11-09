#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C61-DevKitC-1 v2.0 UART0, GPIO4/5 I2C0 BME280, and GPIO8 RGB smoke.
//!
//! This bare-metal image runs on the HP RISC-V core, reads a BME280 through
//! I2C0 on header GPIO4/GPIO5, emits the result through UART0, and sends one
//! red WS2812B frame to the onboard RGB LED on GPIO8. It bypasses the ESP-ROM
//! and second-stage bootloader.

use panic_halt as _;
use riscv_rt::entry;

/// UART0 FIFO at DR_REG_UART0_BASE, from ESP-IDF C61 `uart_reg.h`.
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
/// UART0 PCR registers at DR_REG_PCR_BASE + 0x00/+0x04.
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
const PCR_I2C_CONF: *mut u32 = 0x6009_602C as *mut u32;
/// UART0_CLKDIV_SYNC at UART0 base + 0x14. PLL_F80M / 694 is approximately
/// 115,200 baud for 8N1.
const UART0_CLKDIV_SYNC: *mut u32 = 0x6000_0014 as *mut u32;
/// GPIO8 output path from ESP-IDF C61 `gpio_reg.h`.
const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6009_100C as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1038 as *mut u32;
const GPIO_FUNC8_OUT_SEL_CFG: *mut u32 = 0x6009_1AF4 as *mut u32;
/// IO_MUX GPIO8 register and function selector from ESP-IDF C61 headers.
const IO_MUX_GPIO8: *mut u32 = 0x6009_0020 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;
/// C61 I2C0 GPIO matrix selectors and IO_MUX words for GPIO4/5.
const GPIO_FUNC4_OUT_SEL_CFG: *mut u32 = 0x6009_1AE4 as *mut u32;
const GPIO_FUNC5_OUT_SEL_CFG: *mut u32 = 0x6009_1AE8 as *mut u32;
const GPIO_FUNC46_IN_SEL_CFG: *mut u32 = 0x6009_138C as *mut u32;
const GPIO_FUNC47_IN_SEL_CFG: *mut u32 = 0x6009_1390 as *mut u32;
const IO_MUX_GPIO4: *mut u32 = 0x6009_0010 as *mut u32;
const IO_MUX_GPIO5: *mut u32 = 0x6009_0014 as *mut u32;
const SDA_PIN: u32 = 4;
const SCL_PIN: u32 = 5;
const I2C0_SCL_SIGNAL: u32 = 46;
const I2C0_SDA_SIGNAL: u32 = 47;
const I2C0: usize = 0x6000_4000;
const I2C_FIFO_CONF: *mut u32 = (I2C0 + 0x18) as *mut u32;
const I2C_DATA: *mut u32 = (I2C0 + 0x1C) as *mut u32;
const I2C_CTR: *mut u32 = (I2C0 + 0x04) as *mut u32;
const I2C_INT_RAW: *const u32 = (I2C0 + 0x20) as *const u32;
const I2C_INT_CLR: *mut u32 = (I2C0 + 0x24) as *mut u32;
const I2C_CMD0: usize = I2C0 + 0x58;
const BME280_ADDR: u8 = 0x77;
const OP_RSTART: u32 = 6;
const OP_WRITE: u32 = 1;
const OP_READ: u32 = 3;
const OP_STOP: u32 = 2;
const INT_END_DETECT: u32 = 1 << 3;
const INT_TRANS_COMPLETE: u32 = 1 << 7;
const INT_NACK: u32 = 1 << 10;

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
    uart_byte(HEX[(value & 0x0F) as usize]);
}

fn uart_hex20(value: u32) {
    uart_hex_byte(((value >> 16) & 0xFF) as u8);
    uart_hex_byte(((value >> 8) & 0xFF) as u8);
    uart_hex_byte((value & 0xFF) as u8);
}

#[inline(always)]
unsafe fn i2c_command(index: usize, opcode: u32, bytes: u32) {
    core::ptr::write_volatile(
        (I2C_CMD0 + index * 4) as *mut u32,
        (opcode << 11) | (bytes & 0xFF),
    );
}

/// Read one BME280 register using a write-pointer/repeated-start transaction.
unsafe fn read_bme280_register(register: u8) -> Result<u8, ()> {
    core::ptr::write_volatile(I2C_FIFO_CONF, (1 << 12) | (1 << 13));
    core::ptr::write_volatile(I2C_INT_CLR, u32::MAX);
    i2c_command(0, OP_RSTART, 0);
    i2c_command(1, OP_WRITE, 2);
    i2c_command(2, OP_RSTART, 0);
    i2c_command(3, OP_WRITE, 1);
    i2c_command(4, OP_READ, 1);
    i2c_command(5, OP_STOP, 0);
    core::ptr::write_volatile(I2C_DATA, u32::from(BME280_ADDR << 1));
    core::ptr::write_volatile(I2C_DATA, u32::from(register));
    core::ptr::write_volatile(I2C_DATA, u32::from((BME280_ADDR << 1) | 1));
    core::ptr::write_volatile(I2C_CTR, 1 << 5);

    for _ in 0..200_000 {
        let status = core::ptr::read_volatile(I2C_INT_RAW);
        if status & INT_NACK != 0 {
            return Err(());
        }
        if status & (INT_TRANS_COMPLETE | INT_END_DETECT) != 0 {
            return Ok((core::ptr::read_volatile(I2C_DATA) & 0xFF) as u8);
        }
    }
    Err(())
}

unsafe fn setup_i2c() {
    core::ptr::write_volatile(PCR_I2C_CONF, 1);
    for mux in [IO_MUX_GPIO4, IO_MUX_GPIO5] {
        let value = core::ptr::read_volatile(mux);
        core::ptr::write_volatile(mux, (value & !(0b111 << 12)) | (1 << 12));
    }

    core::ptr::write_volatile(GPIO_FUNC4_OUT_SEL_CFG, I2C0_SDA_SIGNAL | (1 << 10));
    core::ptr::write_volatile(GPIO_FUNC5_OUT_SEL_CFG, I2C0_SCL_SIGNAL | (1 << 10));
    core::ptr::write_volatile(GPIO_FUNC47_IN_SEL_CFG, (1 << 8) | SDA_PIN);
    core::ptr::write_volatile(GPIO_FUNC46_IN_SEL_CFG, (1 << 8) | SCL_PIN);
    core::ptr::write_volatile(GPIO_ENABLE_W1TS, (1 << SDA_PIN) | (1 << SCL_PIN));

    core::ptr::write_volatile((I2C0 + 0x00) as *mut u32, 199);
    core::ptr::write_volatile((I2C0 + 0x30) as *mut u32, 20);
    core::ptr::write_volatile((I2C0 + 0x34) as *mut u32, 20);
    core::ptr::write_volatile((I2C0 + 0x38) as *mut u32, 199);
    core::ptr::write_volatile((I2C0 + 0x40) as *mut u32, 20);
    core::ptr::write_volatile((I2C0 + 0x44) as *mut u32, 20);
    core::ptr::write_volatile((I2C0 + 0x48) as *mut u32, 20);
    core::ptr::write_volatile((I2C0 + 0x4C) as *mut u32, 20);
    core::ptr::write_volatile((I2C0 + 0x54) as *mut u32, 0);
}

unsafe fn smoke_bme280() {
    match read_bme280_register(0xD0) {
        Ok(0x60) => uart_str("BME280_ID=0x60\n"),
        Ok(id) => {
            uart_str("BME280_ID=0x");
            uart_hex_byte(id);
            uart_str("\nBME280_ID_ERROR\n");
            return;
        }
        Err(()) => {
            uart_str("BME280_I2C_ERROR\n");
            return;
        }
    }

    match (
        read_bme280_register(0xFA),
        read_bme280_register(0xFB),
        read_bme280_register(0xFC),
    ) {
        (Ok(msb), Ok(lsb), Ok(xlsb)) => {
            let raw = (u32::from(msb) << 12) | (u32::from(lsb) << 4) | (u32::from(xlsb) >> 4);
            uart_str("BME280_TEMP_RAW=0x");
            uart_hex20(raw);
            uart_str("\n");
        }
        _ => uart_str("BME280_I2C_ERROR\n"),
    }
}

// At 160 MHz these keep the WS2812B high phase below/above the model's 0/1
// threshold while keeping the complete bit cell near 1.25 us.
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

#[inline(always)]
unsafe fn send_ws2812_byte(byte: u8) {
    for bit in (0..8).rev() {
        let one = (byte & (1 << bit)) != 0;
        core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO8_MASK);
        if one {
            long_high();
        } else {
            short_high();
        }
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
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
        // Enable UART0 and select C61's PLL_F80M function-clock source.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, (2 << 20) | (1 << 22));
        core::ptr::write_volatile(UART0_CLKDIV_SYNC, 694);
        uart_str("ESP32_C61_UART0_OK\n");
        setup_i2c();
        smoke_bme280();

        // GPIO8's IO_MUX Function 1 is GPIO8. GPIO_FUNC8_OUT_SEL=256 selects
        // the GPIO_OUT latch and OE_SEL=1 selects GPIO_ENABLE_REG[8].
        let mux = core::ptr::read_volatile(IO_MUX_GPIO8);
        core::ptr::write_volatile(IO_MUX_GPIO8, (mux & !(0b111 << 12)) | (1 << 12));
        core::ptr::write_volatile(GPIO_FUNC8_OUT_SEL_CFG, 256 | (1 << 10));
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);

        // The board's single WS2812B pixel expects GRB order; 00 FF 00 is red.
        send_ws2812_byte(0x00);
        send_ws2812_byte(0xFF);
        send_ws2812_byte(0x00);
        ws2812_reset_gap();
    }

    loop {}
}
