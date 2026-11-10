//! NUCLEO-G491RE + BME280 register-level I2C smoke.
//! Reads BME280.CHIP_ID (0xD0) and raw temperature (0xFA..0xFC) over I2C1
//! on PB8/PB9, then pulses LD2 on PA5 only when both values match.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR1: *mut u32 = 0x4002_1058 as *mut u32;

const GPIOA_BASE: u32 = 0x4800_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;

const GPIOB_BASE: u32 = 0x4800_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_OTYPER: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;
const GPIOB_OSPEEDR: *mut u32 = (GPIOB_BASE + 0x08) as *mut u32;
const GPIOB_PUPDR: *mut u32 = (GPIOB_BASE + 0x0C) as *mut u32;
const GPIOB_AFRH: *mut u32 = (GPIOB_BASE + 0x24) as *mut u32;

const I2C1_BASE: u32 = 0x4000_5400;
const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_TIMINGR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_ISR: *const u32 = (I2C1_BASE + 0x18) as *const u32;
const I2C1_ICR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_RXDR: *const u32 = (I2C1_BASE + 0x24) as *const u32;
const I2C1_TXDR: *mut u32 = (I2C1_BASE + 0x28) as *mut u32;

const LED_MASK: u32 = 1 << 5;
const I2C_ADDRESS: u32 = 0x77;
const REG_CHIP_ID: u8 = 0xD0;
const EXPECTED_CHIP_ID: u8 = 0x60;
const REG_TEMP_MSB: u8 = 0xFA;
const EXPECTED_RAW_TEMP: u32 = 0x08_2D_C0;
const POLL_LIMIT: u32 = 100_000;

const ISR_TXIS: u32 = 1 << 1;
const ISR_RXNE: u32 = 1 << 2;
const ISR_NACKF: u32 = 1 << 4;
const ISR_STOPF: u32 = 1 << 5;
const ISR_TC: u32 = 1 << 6;
const ICR_NACKCF_STOPCF: u32 = (1 << 4) | (1 << 5);

#[entry]
fn main() -> ! {
    unsafe {
        clocks_init();
        gpio_init();
        i2c1_init();
    }

    let chip_id = unsafe { read_register(REG_CHIP_ID) };
    let raw_temp = unsafe { read_raw_temp() };

    if chip_id == Some(EXPECTED_CHIP_ID) && raw_temp == Some(EXPECTED_RAW_TEMP) {
        // The board smoke observes PA5 edges. This loop is entered only after
        // the I2C controller returned both the sensor ID and raw temperature.
        loop {
            led_set(true);
            delay(1_000);
            led_set(false);
            delay(1_000);
        }
    }

    // A missing device, NACK, timeout, or wrong chip ID leaves LD2 low, so the
    // smoke's I2C trace and GPIO-edge assertions fail together.
    loop {
        core::hint::spin_loop();
    }
}

unsafe fn clocks_init() {
    // GPIOA (LD2), GPIOB (I2C1 pads), and I2C1 APB1 clock.
    write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 0b11);
    write_volatile(RCC_APB1ENR1, read_volatile(RCC_APB1ENR1) | (1 << 21));
}

unsafe fn gpio_init() {
    // PA5 / LD2: general-purpose output, initially low.
    let moder_a = read_volatile(GPIOA_MODER);
    write_volatile(GPIOA_MODER, (moder_a & !(0b11 << 10)) | (0b01 << 10));
    led_set(false);

    // PB8/PB9: I2C1 SCL/SDA, alternate function 4, open-drain, high speed.
    let pb8_pb9 = (0b11 << 16) | (0b11 << 18);
    let moder_b = read_volatile(GPIOB_MODER);
    write_volatile(
        GPIOB_MODER,
        (moder_b & !pb8_pb9) | (0b10 << 16) | (0b10 << 18),
    );
    write_volatile(
        GPIOB_OTYPER,
        read_volatile(GPIOB_OTYPER) | (1 << 8) | (1 << 9),
    );
    let speed_b = read_volatile(GPIOB_OSPEEDR);
    write_volatile(GPIOB_OSPEEDR, speed_b | pb8_pb9);
    write_volatile(GPIOB_PUPDR, read_volatile(GPIOB_PUPDR) & !pb8_pb9);
    let afr = read_volatile(GPIOB_AFRH);
    write_volatile(GPIOB_AFRH, (afr & !0xFF) | 0x44);
}

unsafe fn i2c1_init() {
    // ST's STM32CubeG4 NUCLEO-G431RB I2C example uses this TIMINGR value for
    // its 16 MHz I2C kernel clock and standard-mode transactions. The G491RE
    // resets to the same 16 MHz HSI/PCLK1 path used by this smoke.
    write_volatile(I2C1_CR1, 0);
    write_volatile(I2C1_TIMINGR, 0x0030_3D5B);
    write_volatile(I2C1_ICR, ICR_NACKCF_STOPCF);
    write_volatile(I2C1_CR1, 1); // PE
}

unsafe fn read_register(register: u8) -> Option<u8> {
    // Write the BME280 register address, then issue a repeated START for the
    // single-byte read. NBYTES=1 and AUTOEND let the controller generate STOP.
    let write_cr2 = (I2C_ADDRESS << 1) | (1 << 16) | (1 << 13);
    write_volatile(I2C1_CR2, write_cr2);
    if !wait_for(ISR_TXIS) {
        abort_transfer();
        return None;
    }
    write_volatile(I2C1_TXDR, register as u32);
    if !wait_for(ISR_TC) {
        abort_transfer();
        return None;
    }

    let read_cr2 = (I2C_ADDRESS << 1) | (1 << 10) | (1 << 16) | (1 << 25) | (1 << 13);
    write_volatile(I2C1_CR2, read_cr2);
    if !wait_for(ISR_RXNE) {
        abort_transfer();
        return None;
    }
    let value = read_volatile(I2C1_RXDR) as u8;
    if !wait_for(ISR_STOPF) {
        abort_transfer();
        return None;
    }
    write_volatile(I2C1_ICR, ICR_NACKCF_STOPCF);
    Some(value)
}

unsafe fn read_raw_temp() -> Option<u32> {
    let msb = u32::from(read_register(REG_TEMP_MSB)?);
    let lsb = u32::from(read_register(REG_TEMP_MSB + 1)?);
    let xlsb = u32::from(read_register(REG_TEMP_MSB + 2)?);
    // The BME280 exposes a 20-bit ADC result in bits [19:4] of the three-byte
    // register window; the low nibble of XLSB is reserved.
    Some((msb << 12) | (lsb << 4) | (xlsb >> 4))
}

unsafe fn wait_for(flag: u32) -> bool {
    for _ in 0..POLL_LIMIT {
        let isr = read_volatile(I2C1_ISR);
        if isr & ISR_NACKF != 0 {
            return false;
        }
        if isr & flag != 0 {
            return true;
        }
    }
    false
}

unsafe fn abort_transfer() {
    let cr2 = read_volatile(I2C1_CR2);
    write_volatile(I2C1_CR2, cr2 | (1 << 14)); // STOP
    write_volatile(I2C1_ICR, ICR_NACKCF_STOPCF);
}

fn led_set(on: bool) {
    unsafe {
        let value = if on { LED_MASK } else { LED_MASK << 16 };
        write_volatile(GPIOA_BSRR, value);
    }
}

fn delay(iterations: u32) {
    for i in 0..iterations {
        core::hint::black_box(i);
    }
}
