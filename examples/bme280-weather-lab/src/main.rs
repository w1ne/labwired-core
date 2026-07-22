#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;

/// Enable USART1 (APB2 bit 14) and I2C1 (APB1 bit 21). Required now that
/// stm32f103.yaml clocks those peripherals — unclocked MMIO is dropped.
fn enable_peripheral_clocks() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 14)); // USART1EN
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 21)); // I2C1EN
    }
}


const I2C1_BASE: u32 = 0x4000_5400;
const UART1_DR: *mut u8 = (0x4001_3800 + 0x04) as *mut u8;

const I2C1_CR1: *mut u32 = (I2C1_BASE + 0x00) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *const u32 = (I2C1_BASE + 0x14) as *const u32;

// BME280 7-bit address 0x76 → write 0xEC, read 0xED
const BME280_W: u8 = 0xEC;
const BME280_R: u8 = 0xED;

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

fn uart_u32(value: u32) {
    let mut n = value;
    let mut buf = [0u8; 10];
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

fn bme280_write_register(reg: u8, value: u8) {
    i2c_start();
    i2c_write(BME280_W);
    i2c_write(reg);
    i2c_write(value);
    i2c_stop();
}

fn bme280_read_register(reg: u8) -> u8 {
    i2c_start();
    i2c_write(BME280_W);
    i2c_write(reg);
    i2c_start();
    i2c_write(BME280_R);
    let value = i2c_read_byte();
    i2c_stop();
    value
}

fn bme280_read_u16_le(reg: u8) -> u16 {
    let lsb = bme280_read_register(reg) as u16;
    let msb = bme280_read_register(reg + 1) as u16;
    (msb << 8) | lsb
}

#[entry]
fn main() -> ! {
    
    enable_peripheral_clocks();
uart_str("BME280 Weather Lab\n");

    // Check chip ID (register 0xD0 should return 0x60 for BME280)
    let chip_id = bme280_read_register(0xD0);
    uart_str("ChipID=0x");
    uart_hex_u8(chip_id);
    if chip_id == 0x60 {
        uart_str(" BME280 detected\n");
    } else {
        uart_str(" ERR\n");
    }

    // Read temperature calibration coefficients
    let dig_t1 = bme280_read_u16_le(0x88);
    let dig_t2 = bme280_read_u16_le(0x8A) as i16;
    let dig_t3 = bme280_read_u16_le(0x8C) as i16;

    uart_str("T_cal: T1=");
    uart_u32(dig_t1 as u32);
    uart_str(" T2=");
    uart_u32(dig_t2 as u32);
    uart_str(" T3=");
    uart_u32(dig_t3 as u32);
    uart_byte(b'\n');

    // Configure BME280: humidity oversample x1, temp+press oversample x1, normal mode
    bme280_write_register(0xF2, 0x01); // ctrl_hum: hum oversample x1
    bme280_write_register(0xF4, 0x27); // ctrl_meas: temp+press oversample x1, normal mode

    loop {
        // Read raw press (0xF7..0xF9), temp (0xFA..0xFC), hum (0xFD..0xFE)
        let press_msb = bme280_read_register(0xF7) as u32;
        let press_lsb = bme280_read_register(0xF8) as u32;
        let press_xlsb = bme280_read_register(0xF9) as u32;
        let temp_msb = bme280_read_register(0xFA) as u32;
        let temp_lsb = bme280_read_register(0xFB) as u32;
        let temp_xlsb = bme280_read_register(0xFC) as u32;
        let hum_msb = bme280_read_register(0xFD) as u32;
        let hum_lsb = bme280_read_register(0xFE) as u32;

        // Reconstruct 20-bit ADC values (upper bits of 3-byte fields)
        let press_raw = (press_msb << 12) | (press_lsb << 4) | (press_xlsb >> 4);
        let temp_raw = (temp_msb << 12) | (temp_lsb << 4) | (temp_xlsb >> 4);
        let hum_raw = (hum_msb << 8) | hum_lsb;

        // Print raw ADC values — compensation math: see Bosch BME280 datasheet section 4.2.3
        uart_str("T_raw=");
        uart_u32(temp_raw);
        uart_str(" P_raw=");
        uart_u32(press_raw);
        uart_str(" H_raw=");
        uart_u32(hum_raw);
        uart_byte(b'\n');

        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
