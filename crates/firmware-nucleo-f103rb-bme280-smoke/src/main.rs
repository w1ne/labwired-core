// NUCLEO-F103RB + BME280 register-level smoke (Cortex-M3, 8 MHz reset HSI).
//
// Reads BME280.CHIP_ID over I2C1, remapped to Arduino D15/D14 (PB8/PB9),
// sends the result through USART2/ST-LINK VCP, and pulses LD2 only on ID 0x60.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_101C as *mut u32;

const GPIOA_BASE: u32 = 0x4001_0800;
const GPIOA_CRL: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x10) as *mut u32;

const GPIOB_BASE: u32 = 0x4001_0C00;
const GPIOB_CRH: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;

const AFIO_MAPR: *mut u32 = 0x4001_0004 as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR: *const u32 = USART2_BASE as *const u32;
const USART2_DR: *mut u32 = (USART2_BASE + 0x04) as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x08) as *mut u32;
const USART2_CR1: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;

const I2C1_BASE: u32 = 0x4000_5400;
const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_OAR1: *mut u32 = (I2C1_BASE + 0x08) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_SR1: *const u32 = (I2C1_BASE + 0x14) as *const u32;
const I2C1_SR2: *const u32 = (I2C1_BASE + 0x18) as *const u32;
const I2C1_CCR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_TRISE: *mut u32 = (I2C1_BASE + 0x20) as *mut u32;

const LED_PIN: u32 = 5;
const BME280_ADDRESS: u8 = 0x77;
const POLL_LIMIT: u32 = 100_000;

const I2C_CR1_PE: u32 = 1 << 0;
const I2C_CR1_START: u32 = 1 << 8;
const I2C_CR1_STOP: u32 = 1 << 9;
const I2C_CR1_ACK: u32 = 1 << 10;
const I2C_SR1_SB: u32 = 1 << 0;
const I2C_SR1_ADDR: u32 = 1 << 1;
const I2C_SR1_BTF: u32 = 1 << 2;
const I2C_SR1_RXNE: u32 = 1 << 6;
const I2C_SR1_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        clock_init();
        gpio_init();
        uart_init();
        i2c1_init();
    }

    match unsafe { bme280_chip_id() } {
        Some(0x60) => {
            puts(b"NUCLEO_F103RB_BME280_OK ID=0x60 I2C1=PB8/PB9\n");
            for _ in 0..16 {
                led_set(true);
                delay(4_000);
                led_set(false);
                delay(4_000);
            }
        }
        Some(id) => {
            puts(b"NUCLEO_F103RB_BME280_BAD_ID=0x");
            put_hex(id);
            putc(b'\n');
        }
        None => puts(b"NUCLEO_F103RB_BME280_I2C_ERROR\n"),
    }

    loop {
        core::hint::spin_loop();
    }
}

unsafe fn clock_init() {
    // GPIOA, GPIOB and AFIO on APB2; USART2 and I2C1 on APB1.
    write_volatile(RCC_APB2ENR, (1 << 0) | (1 << 2) | (1 << 3));
    write_volatile(RCC_APB1ENR, (1 << 17) | (1 << 21));

    // NUCLEO-F103RB Arduino D15/D14 use the remapped I2C1 pins PB8/PB9.
    let mapr = read_volatile(AFIO_MAPR);
    write_volatile(AFIO_MAPR, mapr | (1 << 1));
}

unsafe fn gpio_init() {
    // PA2 USART2_TX = AF push-pull 50 MHz; PA3 RX = floating input;
    // PA5 LD2 = general-purpose push-pull output 2 MHz.
    let mut crl = read_volatile(GPIOA_CRL);
    crl = (crl & !(0xF << 8)) | (0xB << 8);
    crl = (crl & !(0xF << 12)) | (0x4 << 12);
    crl = (crl & !(0xF << 20)) | (0x2 << 20);
    write_volatile(GPIOA_CRL, crl);

    // PB8/SCL and PB9/SDA = alternate-function open-drain 50 MHz.
    let crh = read_volatile(GPIOB_CRH);
    write_volatile(GPIOB_CRH, (crh & !0xFF) | 0xFF);
}

unsafe fn uart_init() {
    // Reset HSI is 8 MHz: BRR = 8 MHz / 115200, rounded to 0x45.
    write_volatile(USART2_BRR, 0x45);
    write_volatile(USART2_CR1, (1 << 13) | (1 << 3)); // UE | TE
}

unsafe fn i2c1_init() {
    // APB1 reset clock is 8 MHz. Standard-mode 100 kHz timing.
    write_volatile(I2C1_CR2, 8); // FREQ = 8 MHz
    write_volatile(I2C1_CCR, 40); // PCLK1 / (2 * 100 kHz)
    write_volatile(I2C1_TRISE, 9); // 8 MHz + 1
    write_volatile(I2C1_OAR1, 1 << 14); // Required reserved bit
    write_volatile(I2C1_CR1, I2C_CR1_PE);
}

unsafe fn bme280_chip_id() -> Option<u8> {
    if !i2c_start()
        || !i2c_send_address(BME280_ADDRESS, false)
        || !i2c_write_byte(0xD0)
        || !i2c_start()
        || !i2c_send_address(BME280_ADDRESS, true)
    {
        i2c_stop();
        return None;
    }

    let id = i2c_read_byte_nack()?;
    i2c_stop();
    Some(id)
}

unsafe fn i2c_start() -> bool {
    let cr1 = read_volatile(I2C1_CR1);
    write_volatile(I2C1_CR1, cr1 | I2C_CR1_START);
    poll_sr1(I2C_SR1_SB)
}

unsafe fn i2c_send_address(address: u8, read: bool) -> bool {
    let byte = (address << 1) | u8::from(read);
    write_volatile(I2C1_DR, u32::from(byte));
    if !poll_sr1(I2C_SR1_ADDR) {
        return false;
    }
    // The F1 sequence clears ADDR by reading SR1 (done by poll_sr1) then SR2.
    let _ = read_volatile(I2C1_SR2);
    true
}

unsafe fn i2c_write_byte(byte: u8) -> bool {
    if !poll_sr1(I2C_SR1_TXE) {
        return false;
    }
    write_volatile(I2C1_DR, u32::from(byte));
    poll_sr1(I2C_SR1_BTF)
}

unsafe fn i2c_read_byte_nack() -> Option<u8> {
    let cr1 = read_volatile(I2C1_CR1);
    write_volatile(I2C1_CR1, (cr1 & !I2C_CR1_ACK) | I2C_CR1_STOP);
    if !poll_sr1(I2C_SR1_RXNE) {
        return None;
    }
    Some(read_volatile(I2C1_DR) as u8)
}

unsafe fn i2c_stop() {
    let cr1 = read_volatile(I2C1_CR1);
    write_volatile(I2C1_CR1, cr1 | I2C_CR1_STOP);
}

unsafe fn poll_sr1(mask: u32) -> bool {
    for _ in 0..POLL_LIMIT {
        if read_volatile(I2C1_SR1) & mask != 0 {
            return true;
        }
    }
    false
}

fn led_set(on: bool) {
    let value = if on {
        1 << LED_PIN
    } else {
        1 << (LED_PIN + 16)
    };
    unsafe { write_volatile(GPIOA_BSRR, value) };
}

fn puts(bytes: &[u8]) {
    for &byte in bytes {
        putc(byte);
    }
}

fn putc(byte: u8) {
    unsafe {
        for _ in 0..POLL_LIMIT {
            if read_volatile(USART2_SR) & (1 << 7) != 0 {
                break;
            }
        }
        write_volatile(USART2_DR, u32::from(byte));
    }
}

fn put_hex(value: u8) {
    for nibble in [value >> 4, value & 0x0F] {
        putc(if nibble < 10 {
            b'0' + nibble
        } else {
            b'A' + nibble - 10
        });
    }
}

fn delay(count: u32) {
    for _ in 0..count {
        core::hint::spin_loop();
    }
}
