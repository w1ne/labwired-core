// Bare-metal NUCLEO-H533RE smoke: BME280 chip ID over I2C1, USART2 through
// ST-LINK VCP, and LD2 on PA5. Routes follow RM0481 and MB1814 C-02.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4402_0C00;
const RCC_AHB2ENR: *mut u32 = (RCC_BASE + 0x8C) as *mut u32;
const RCC_APB1LENR: *mut u32 = (RCC_BASE + 0x9C) as *mut u32;

const GPIOA_BASE: u32 = 0x4202_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;

const GPIOB_BASE: u32 = 0x4202_0400;
const GPIOB_MODER: *mut u32 = GPIOB_BASE as *mut u32;
const GPIOB_OTYPER: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;
const GPIOB_OSPEEDR: *mut u32 = (GPIOB_BASE + 0x08) as *mut u32;
const GPIOB_PUPDR: *mut u32 = (GPIOB_BASE + 0x0C) as *mut u32;
const GPIOB_AFRL: *mut u32 = (GPIOB_BASE + 0x20) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u8 = (USART2_BASE + 0x28) as *mut u8;

const I2C1_BASE: u32 = 0x4000_5400;
const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_TIMINGR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_ISR: *const u32 = (I2C1_BASE + 0x18) as *const u32;
const I2C1_ICR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_RXDR: *const u8 = (I2C1_BASE + 0x24) as *const u8;
const I2C1_TXDR: *mut u8 = (I2C1_BASE + 0x28) as *mut u8;

const TXE: u32 = 1 << 7;
const LED_PA5: u32 = 5;
const I2C_TXIS: u32 = 1 << 1;
const I2C_RXNE: u32 = 1 << 2;
const I2C_NACKF: u32 = 1 << 4;
const I2C_STOPF: u32 = 1 << 5;
const I2C_TC: u32 = 1 << 6;
const I2C_POLL_LIMIT: u32 = 100_000;
const BME280_ADDRESS: u32 = 0x77;

#[entry]
fn main() -> ! {
    unsafe {
        // H5 reset clock is 32 MHz HSI/2; enable GPIOA/B, USART2 and I2C1.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 1);
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | (1 << 1));
        write_volatile(
            RCC_APB1LENR,
            read_volatile(RCC_APB1LENR) | (1 << 17) | (1 << 21),
        );

        // PA5 / LD2 is an active-high push-pull output.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !(0b11 << (LED_PA5 * 2))) | (0b01 << (LED_PA5 * 2)),
        );

        // USART2 VCP is routed to PA2/PA3, alternate function 7.
        let moder = read_volatile(GPIOA_MODER);
        let uart_pins = (0b11 << (2 * 2)) | (0b11 << (3 * 2));
        write_volatile(
            GPIOA_MODER,
            (moder & !uart_pins) | (0b10 << (2 * 2)) | (0b10 << (3 * 2)),
        );
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(GPIOA_AFRL, (afrl & !0x0000_FF00) | 0x0000_7700);

        // 32 MHz PCLK / 115200 baud, 8 data bits, no parity, one stop bit.
        write_volatile(USART2_BRR, 278);
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE

        // PB6/D15 = I2C1_SCL and PB7/D14 = I2C1_SDA (AF4), open-drain.
        let moder = read_volatile(GPIOB_MODER);
        let pins = (0b11 << (6 * 2)) | (0b11 << (7 * 2));
        write_volatile(
            GPIOB_MODER,
            (moder & !pins) | (0b10 << (6 * 2)) | (0b10 << (7 * 2)),
        );
        write_volatile(
            GPIOB_OTYPER,
            read_volatile(GPIOB_OTYPER) | (1 << 6) | (1 << 7),
        );
        let speedr = read_volatile(GPIOB_OSPEEDR);
        write_volatile(
            GPIOB_OSPEEDR,
            (speedr & !pins) | (0b10 << (6 * 2)) | (0b10 << (7 * 2)),
        );
        let pupdr = read_volatile(GPIOB_PUPDR);
        write_volatile(GPIOB_PUPDR, pupdr & !pins); // breakout supplies pull-ups
        let afrl = read_volatile(GPIOB_AFRL);
        write_volatile(GPIOB_AFRL, (afrl & !0xFF00_0000) | 0x4400_0000);

        // I2C v2 peripheral, modelled H5 layout. The local smoke verifies the
        // register transaction, not the physical SCL frequency on silicon.
        write_volatile(I2C1_TIMINGR, 0x0070_3031);
        write_volatile(I2C1_CR1, 1); // PE
    }

    match bme280_chip_id() {
        Some(0x60) => send(b"NUCLEO_H533RE_BME280_OK ID=0x60 I2C1=PB6/PB7\n"),
        Some(_) => send(b"NUCLEO_H533RE_BME280_BAD_ID\n"),
        None => send(b"NUCLEO_H533RE_BME280_I2C_ERROR\n"),
    }
    loop {
        led_set(true);
        delay(10_000);
        led_set(false);
        delay(10_000);
    }
}

/// Read BME280.CHIP_ID (0xD0) using a repeated-start register read.
fn bme280_chip_id() -> Option<u8> {
    unsafe {
        // Write the register pointer and leave the bus owned for the restart.
        write_volatile(
            I2C1_CR2,
            (BME280_ADDRESS << 1) | (1 << 16) | (1 << 13), // SADD, NBYTES=1, START
        );
        if !wait_i2c(I2C_TXIS) {
            return None;
        }
        write_volatile(I2C1_TXDR, 0xD0);
        if !wait_i2c(I2C_TC) {
            return None;
        }

        // Repeated START followed by one-byte AUTOEND read.
        write_volatile(
            I2C1_CR2,
            (BME280_ADDRESS << 1) | (1 << 10) | (1 << 16) | (1 << 25) | (1 << 13),
        );
        if !wait_i2c(I2C_RXNE) {
            return None;
        }
        let id = read_volatile(I2C1_RXDR);
        if !wait_i2c(I2C_STOPF) {
            return None;
        }
        write_volatile(I2C1_ICR, I2C_STOPF);
        Some(id)
    }
}

fn wait_i2c(flag: u32) -> bool {
    for _ in 0..I2C_POLL_LIMIT {
        let status = unsafe { read_volatile(I2C1_ISR) };
        if status & I2C_NACKF != 0 {
            unsafe { write_volatile(I2C1_ICR, I2C_NACKF | I2C_STOPF) };
            return false;
        }
        if status & flag != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn led_set(on: bool) {
    let value = if on {
        1 << LED_PA5
    } else {
        1 << (LED_PA5 + 16)
    };
    unsafe { write_volatile(GPIOA_BSRR, value) };
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            while read_volatile(USART2_ISR) & TXE == 0 {
                core::hint::spin_loop();
            }
            write_volatile(USART2_TDR, *byte);
        }
    }
}

fn delay(count: u32) {
    for _ in 0..count {
        core::hint::spin_loop();
    }
}
