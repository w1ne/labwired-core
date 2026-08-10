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
    usart1_init();
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
