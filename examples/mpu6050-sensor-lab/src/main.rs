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

// MPU6050 7-bit address 0x68 → write 0xD0, read 0xD1
const MPU6050_W: u8 = 0xD0;
const MPU6050_R: u8 = 0xD1;

fn uart_byte(byte: u8) {
    unsafe { core::ptr::write_volatile(UART1_DR, byte) }
}

fn uart_str(value: &str) {
    for byte in value.bytes() {
        uart_byte(byte);
    }
}

fn uart_i16(value: i16) {
    if value < 0 {
        uart_byte(b'-');
    }
    let mut n = if value < 0 {
        value.wrapping_neg() as u16
    } else {
        value as u16
    };
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

fn uart_hex_u8(value: u8) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    uart_byte(HEX[(value >> 4) as usize]);
    uart_byte(HEX[(value & 0xF) as usize]);
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

fn mpu6050_write_register(reg: u8, value: u8) {
    i2c_start();
    i2c_write(MPU6050_W);
    i2c_write(reg);
    i2c_write(value);
    i2c_stop();
}

fn mpu6050_read_register(reg: u8) -> u8 {
    i2c_start();
    i2c_write(MPU6050_W);
    i2c_write(reg);
    i2c_start();
    i2c_write(MPU6050_R);
    let value = i2c_read_byte();
    i2c_stop();
    value
}

fn read_i16_be(hi_reg: u8) -> i16 {
    let hi = mpu6050_read_register(hi_reg) as u16;
    let lo = mpu6050_read_register(hi_reg + 1) as u16;
    ((hi << 8) | lo) as i16
}

#[entry]
fn main() -> ! {
    
    enable_peripheral_clocks();
uart_str("MPU6050 IMU Lab\n");

    // Wake MPU6050: clear SLEEP bit in PWR_MGMT_1 (reg 0x6B)
    mpu6050_write_register(0x6B, 0x00);

    // Read WHO_AM_I (reg 0x75) — should return 0x68
    let who_am_i = mpu6050_read_register(0x75);
    uart_str("WHO_AM_I=0x");
    uart_hex_u8(who_am_i);
    if who_am_i == 0x68 {
        uart_str(" OK\n");
    } else {
        uart_str(" ERR\n");
    }

    loop {
        // Accel: registers 0x3B(AX_H), 0x3C(AX_L), 0x3D(AY_H), 0x3E(AY_L), 0x3F(AZ_H), 0x40(AZ_L)
        let ax = read_i16_be(0x3B);
        let ay = read_i16_be(0x3D);
        let az = read_i16_be(0x3F);

        // Gyro: registers 0x43(GX_H), 0x44(GX_L), 0x45(GY_H), 0x46(GY_L), 0x47(GZ_H), 0x48(GZ_L)
        let gx = read_i16_be(0x43);
        let gy = read_i16_be(0x45);
        let gz = read_i16_be(0x47);

        uart_str("AX=");
        uart_i16(ax);
        uart_str(" AY=");
        uart_i16(ay);
        uart_str(" AZ=");
        uart_i16(az);
        uart_str(" GX=");
        uart_i16(gx);
        uart_str(" GY=");
        uart_i16(gy);
        uart_str(" GZ=");
        uart_i16(gz);
        uart_byte(b'\n');

        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
