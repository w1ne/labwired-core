//! NUCLEO-G431KB + BME280 register-level smoke.
//! Reads CHIP_ID and raw temperature over I2C1 (PA15/PB7), reports the result
//! over the board's USART2 ST-LINK VCP, then pulses user LED LD2 on PB8.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR1: *mut u32 = 0x4002_1058 as *mut u32;

const GPIOA_BASE: u32 = 0x4800_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_OTYPER: *mut u32 = (GPIOA_BASE + 0x04) as *mut u32;
const GPIOA_OSPEEDR: *mut u32 = (GPIOA_BASE + 0x08) as *mut u32;
const GPIOA_PUPDR: *mut u32 = (GPIOA_BASE + 0x0C) as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_AFRH: *mut u32 = (GPIOA_BASE + 0x24) as *mut u32;

const GPIOB_BASE: u32 = 0x4800_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_OTYPER: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;
const GPIOB_OSPEEDR: *mut u32 = (GPIOB_BASE + 0x08) as *mut u32;
const GPIOB_PUPDR: *mut u32 = (GPIOB_BASE + 0x0C) as *mut u32;
const GPIOB_AFRL: *mut u32 = (GPIOB_BASE + 0x20) as *mut u32;
const GPIOB_BSRR: *mut u32 = (GPIOB_BASE + 0x18) as *mut u32;

const I2C1_BASE: u32 = 0x4000_5400;
const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_TIMINGR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_ISR: *const u32 = (I2C1_BASE + 0x18) as *const u32;
const I2C1_ICR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_RXDR: *const u32 = (I2C1_BASE + 0x24) as *const u32;
const I2C1_TXDR: *mut u32 = (I2C1_BASE + 0x28) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const LED_MASK: u32 = 1 << 8;
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
const ISR_TXE_TXFNF: u32 = 1 << 7;
const ICR_NACKCF_STOPCF: u32 = (1 << 4) | (1 << 5);

#[entry]
fn main() -> ! {
    unsafe {
        clocks_init();
        gpio_init();
        i2c1_init();
        usart2_init();
    }

    let chip_id = unsafe { read_register(REG_CHIP_ID) };
    let raw_temp = unsafe { read_raw_temp() };
    if chip_id == Some(EXPECTED_CHIP_ID) && raw_temp == Some(EXPECTED_RAW_TEMP) {
        unsafe { usart2_write(b"G431KB_BME280_OK id=0x60 temp=0x082DC0\n") };
        loop {
            led_set(true);
            delay(1_000);
            led_set(false);
            delay(1_000);
        }
    }

    unsafe { usart2_write(b"G431KB_BME280_FAIL\n") };
    loop {
        core::hint::spin_loop();
    }
}

unsafe fn clocks_init() {
    // GPIOA/GPIOB, USART2 and I2C1.
    write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 0b11);
    write_volatile(
        RCC_APB1ENR1,
        read_volatile(RCC_APB1ENR1) | (1 << 17) | (1 << 21),
    );
}

unsafe fn gpio_init() {
    // LD2 is PB8 on the NUCLEO-G431KB default bridge configuration.
    let led_mode = 0b11 << 16;
    write_volatile(
        GPIOB_MODER,
        (read_volatile(GPIOB_MODER) & !led_mode) | (0b01 << 16),
    );
    led_set(false);

    // USART2 VCP: PA2 TX and PA3 RX, AF7.
    let uart_mode = 0b1111 << 4;
    write_volatile(
        GPIOA_MODER,
        (read_volatile(GPIOA_MODER) & !uart_mode) | (0b1010 << 4),
    );
    let uart_af = (0xF << 8) | (0xF << 12);
    write_volatile(
        GPIOA_AFRL,
        (read_volatile(GPIOA_AFRL) & !uart_af) | (0x7 << 8) | (0x7 << 12),
    );

    // I2C1: PA15 SCL and PB7 SDA, AF4, open-drain.
    let scl_mode = 0b11 << 30;
    write_volatile(
        GPIOA_MODER,
        (read_volatile(GPIOA_MODER) & !scl_mode) | (0b10 << 30),
    );
    write_volatile(GPIOA_OTYPER, read_volatile(GPIOA_OTYPER) | (1 << 15));
    write_volatile(GPIOA_OSPEEDR, read_volatile(GPIOA_OSPEEDR) | (0b11 << 30));
    write_volatile(GPIOA_PUPDR, read_volatile(GPIOA_PUPDR) & !scl_mode);
    write_volatile(
        GPIOA_AFRH,
        (read_volatile(GPIOA_AFRH) & !(0xF << 28)) | (0x4 << 28),
    );

    let sda_mode = 0b11 << 14;
    write_volatile(
        GPIOB_MODER,
        (read_volatile(GPIOB_MODER) & !sda_mode) | (0b10 << 14),
    );
    write_volatile(GPIOB_OTYPER, read_volatile(GPIOB_OTYPER) | (1 << 7));
    write_volatile(GPIOB_OSPEEDR, read_volatile(GPIOB_OSPEEDR) | (0b11 << 14));
    write_volatile(GPIOB_PUPDR, read_volatile(GPIOB_PUPDR) & !sda_mode);
    write_volatile(
        GPIOB_AFRL,
        (read_volatile(GPIOB_AFRL) & !(0xF << 28)) | (0x4 << 28),
    );
}

unsafe fn i2c1_init() {
    // Reset HSI/PCLK1 is 16 MHz; this value configures standard-mode I2C.
    write_volatile(I2C1_CR1, 0);
    write_volatile(I2C1_TIMINGR, 0x0030_3D5B);
    write_volatile(I2C1_ICR, ICR_NACKCF_STOPCF);
    write_volatile(I2C1_CR1, 1); // PE
}

unsafe fn usart2_init() {
    write_volatile(USART2_CR1, 0);
    write_volatile(USART2_BRR, 139); // 16 MHz / 115200 baud, rounded.
    write_volatile(USART2_CR1, (1 << 3) | 1); // TE | UE
}

unsafe fn usart2_write(bytes: &[u8]) {
    for byte in bytes {
        for _ in 0..POLL_LIMIT {
            if read_volatile(USART2_ISR) & ISR_TXE_TXFNF != 0 {
                break;
            }
        }
        write_volatile(USART2_TDR, u32::from(*byte));
    }
}

unsafe fn read_register(register: u8) -> Option<u8> {
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
        write_volatile(GPIOB_BSRR, value);
    }
}

fn delay(iterations: u32) {
    for i in 0..iterations {
        core::hint::black_box(i);
    }
}
