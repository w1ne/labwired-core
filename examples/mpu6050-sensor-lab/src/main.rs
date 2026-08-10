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
    usart1_init();
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
