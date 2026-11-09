#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB2ENR: *mut u32 = 0x4402_0C8C as *mut u32;
const RCC_APB1LENR: *mut u32 = 0x4402_0C9C as *mut u32;

const GPIOA_MODER: *mut u32 = 0x4202_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4202_0018 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4202_0400 as *mut u32;
const GPIOB_OTYPER: *mut u32 = 0x4202_0404 as *mut u32;
const GPIOB_AFRL: *mut u32 = 0x4202_0420 as *mut u32;

const I2C1_CR1: *mut u32 = 0x4000_5400 as *mut u32;
const I2C1_CR2: *mut u32 = 0x4000_5404 as *mut u32;
const I2C1_TIMINGR: *mut u32 = 0x4000_5410 as *mut u32;
const I2C1_ISR: *const u32 = 0x4000_5418 as *const u32;
const I2C1_ICR: *mut u32 = 0x4000_541C as *mut u32;
const I2C1_RXDR: *const u32 = 0x4000_5424 as *const u32;
const I2C1_TXDR: *mut u32 = 0x4000_5428 as *mut u32;

const LED_LD2: u32 = 1 << 5;
const ISR_TXIS: u32 = 1 << 1;
const ISR_RXNE: u32 = 1 << 2;
const ISR_NACKF: u32 = 1 << 4;
const ISR_STOPF: u32 = 1 << 5;
const ISR_TC: u32 = 1 << 6;
const ICR_NACKCF: u32 = 1 << 4;
const ICR_STOPCF: u32 = 1 << 5;
const BME280_ADDRESS: u32 = 0x77;

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

        // Write the CHIP_ID register address, then issue a repeated START.
        write_volatile(I2C1_CR2, address | (1 << 16) | (1 << 13));
        if !wait_for(ISR_TXIS) {
            return Err(());
        }
        write_volatile(I2C1_TXDR, 0xD0);
        if !wait_for(ISR_TC) {
            return Err(());
        }

        // One-byte read with AUTOEND. The BME280's expected ID is 0x60.
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

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA, GPIOB, and I2C1 on the H5 reset clock tree.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | (1 << 0) | (1 << 1));
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 21));

        // PA5 is the NUCLEO-H533RE active-high LD2.
        let moder_a = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder_a & !(0b11 << 10)) | (0b01 << 10));
        write_volatile(GPIOA_BSRR, LED_LD2 << 16);

        // Arduino D15/D14 are PB6/SCL and PB7/SDA (AF4, open-drain).
        let moder_b = read_volatile(GPIOB_MODER);
        write_volatile(
            GPIOB_MODER,
            (moder_b & !((0b11 << 12) | (0b11 << 14))) | (0b10 << 12) | (0b10 << 14),
        );
        let otyper_b = read_volatile(GPIOB_OTYPER);
        write_volatile(GPIOB_OTYPER, otyper_b | (1 << 6) | (1 << 7));
        let afrl_b = read_volatile(GPIOB_AFRL);
        write_volatile(
            GPIOB_AFRL,
            (afrl_b & !((0xF << 24) | (0xF << 28))) | (4 << 24) | (4 << 28),
        );

        // HSI64/PCLK1 64 MHz: PRESC=15 and 20 SCL ticks per phase gives
        // approximately 100 kHz, with SCLDEL=1.25 us and SDADEL=0.
        write_volatile(I2C1_TIMINGR, 0xF041_1313);
        write_volatile(I2C1_CR1, 1); // PE
    }

    // A successful ID read is observable on the board LED: 16 high/low pulses.
    // A missing device, NACK, timeout, or wrong ID leaves LD2 low, so the smoke
    // cannot pass just because the firmware reached its main loop.
    if read_bme280_id() == Ok(0x60) {
        for _ in 0..16 {
            unsafe {
                write_volatile(GPIOA_BSRR, LED_LD2);
                write_volatile(GPIOA_BSRR, LED_LD2 << 16);
            }
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
