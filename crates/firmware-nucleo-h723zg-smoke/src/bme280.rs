#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB4ENR: *mut u32 = 0x5802_44E0 as *mut u32;
const RCC_APB1LENR: *mut u32 = 0x5802_44E8 as *mut u32;

const GPIOB: u32 = 0x5802_0400;
const GPIOD: u32 = 0x5802_0C00;
const MODER: u32 = 0x00;
const OTYPER: u32 = 0x04;
const BSRR: u32 = 0x18;
const AFRH: u32 = 0x24;

const USART3_BRR: *mut u32 = 0x4000_480C as *mut u32;
const USART3_CR1: *mut u32 = 0x4000_4800 as *mut u32;
const USART3_ISR: *const u32 = 0x4000_481C as *const u32;
const USART3_TDR: *mut u32 = 0x4000_4828 as *mut u32;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE_TE: u32 = (1 << 0) | (1 << 3);

const I2C1_CR1: *mut u32 = 0x4000_5400 as *mut u32;
const I2C1_CR2: *mut u32 = 0x4000_5404 as *mut u32;
const I2C1_TIMINGR: *mut u32 = 0x4000_5410 as *mut u32;
const I2C1_ISR: *const u32 = 0x4000_5418 as *const u32;
const I2C1_ICR: *mut u32 = 0x4000_541C as *mut u32;
const I2C1_RXDR: *const u32 = 0x4000_5424 as *const u32;
const I2C1_TXDR: *mut u32 = 0x4000_5428 as *mut u32;

const LED_LD1_PB0: u32 = 1;
const ISR_TXIS: u32 = 1 << 1;
const ISR_RXNE: u32 = 1 << 2;
const ISR_NACKF: u32 = 1 << 4;
const ISR_STOPF: u32 = 1 << 5;
const ISR_TC: u32 = 1 << 6;
const ICR_NACKCF: u32 = 1 << 4;
const ICR_STOPCF: u32 = 1 << 5;
const BME280_ADDRESS: u32 = 0x77;

fn reg_rmw(address: u32, clear: u32, set: u32) {
    unsafe {
        let register = address as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

fn wait_for(mask: u32) -> bool {
    for _ in 0..500_000 {
        let status = unsafe { read_volatile(I2C1_ISR) };
        if status & mask != 0 {
            return true;
        }
        if status & ISR_NACKF != 0 {
            unsafe { write_volatile(I2C1_ICR, ICR_NACKCF | ICR_STOPCF) };
            return false;
        }
    }
    false
}

fn read_bme280_id() -> Result<u8, ()> {
    unsafe {
        let address = BME280_ADDRESS << 1;

        // Select CHIP_ID (0xD0), then issue a repeated START for one byte.
        write_volatile(I2C1_CR2, address | (1 << 16) | (1 << 13));
        if !wait_for(ISR_TXIS) {
            return Err(());
        }
        write_volatile(I2C1_TXDR, 0xD0);
        if !wait_for(ISR_TC) {
            return Err(());
        }

        // One-byte read with AUTOEND. BME280's expected chip ID is 0x60.
        write_volatile(
            I2C1_CR2,
            address | (1 << 10) | (1 << 16) | (1 << 25) | (1 << 13),
        );
        if !wait_for(ISR_RXNE) {
            return Err(());
        }
        let id = read_volatile(I2C1_RXDR) as u8;
        if !wait_for(ISR_STOPF) {
            return Err(());
        }
        write_volatile(I2C1_ICR, ICR_STOPCF);
        Ok(id)
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART3_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(USART3_TDR, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for &byte in text {
        putc(byte);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Use reset HSI64 and enable GPIOB/GPIOD, USART3, and I2C1.
        write_volatile(RCC_AHB4ENR, (1 << 1) | (1 << 3));
        write_volatile(
            RCC_APB1LENR,
            read_volatile(RCC_APB1LENR) | (1 << 18) | (1 << 21),
        );

        // LD1 is active-high on PB0. PB8/PB9 are I2C1 SCL/SDA (AF4, open-drain).
        reg_rmw(
            GPIOB + MODER,
            (0b11 << 0) | (0b11 << 16) | (0b11 << 18),
            (0b01 << 0) | (0b10 << 16) | (0b10 << 18),
        );
        reg_rmw(GPIOB + OTYPER, 0, (1 << 8) | (1 << 9));
        reg_rmw(GPIOB + AFRH, 0xFF, (4 << 0) | (4 << 4));
        write_volatile((GPIOB + BSRR) as *mut u32, LED_LD1_PB0 << 16);

        // USART3 TX is PD8/AF7 on the ST-LINK virtual COM route.
        reg_rmw(GPIOD + MODER, 0b11 << 16, 0b10 << 16);
        reg_rmw(GPIOD + AFRH, 0xF, 0x7);
        write_volatile(USART3_BRR, 556); // 64 MHz / 115200 baud, rounded.
        write_volatile(USART3_CR1, USART_CR1_UE_TE);

        // HSI64/PCLK1 64 MHz: approximately 100 kHz I2C, using I2C v2 timing.
        write_volatile(I2C1_TIMINGR, 0xF041_1313);
        write_volatile(I2C1_CR1, 1); // PE
    }

    if read_bme280_id() == Ok(0x60) {
        for _ in 0..16 {
            unsafe {
                write_volatile((GPIOB + BSRR) as *mut u32, LED_LD1_PB0);
                write_volatile((GPIOB + BSRR) as *mut u32, LED_LD1_PB0 << 16);
            }
        }
        puts(b"NUCLEO_H723ZG_BME280_OK ID=0x60 I2C1=PB8/PB9\n");
    } else {
        puts(b"NUCLEO_H723ZG_BME280_FAIL\n");
    }

    loop {
        core::hint::spin_loop();
    }
}
