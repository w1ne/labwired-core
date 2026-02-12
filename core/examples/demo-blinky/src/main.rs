#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// GPIO Base Addresses
const GPIOC_BASE: u32 = 0x40011000;
const RCC_BASE: u32 = 0x40021000;
const I2C1_BASE: u32 = 0x40005400;

// RCC Registers
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;

// GPIO Registers
const GPIOC_CRH: *mut u32 = (GPIOC_BASE + 0x04) as *mut u32;
const GPIOC_ODR: *mut u32 = (GPIOC_BASE + 0x0C) as *mut u32;

// I2C Registers
const I2C1_CR1: *mut u32 = I2C1_BASE as *mut u32;
const I2C1_CR2: *mut u32 = (I2C1_BASE + 0x04) as *mut u32;
const I2C1_DR: *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_CCR: *mut u32 = (I2C1_BASE + 0x1C) as *mut u32;
const I2C1_TRISE: *mut u32 = (I2C1_BASE + 0x20) as *mut u32;

// Bit definitions
const RCC_APB2ENR_IOPCEN: u32 = 1 << 4;
const RCC_APB1ENR_I2C1EN: u32 = 1 << 21;
const GPIO_ODR_ODR13: u32 = 1 << 13;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOC and I2C1 clocks
        let rcc_apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, rcc_apb2 | RCC_APB2ENR_IOPCEN);

        let rcc_apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, rcc_apb1 | RCC_APB1ENR_I2C1EN);

        // Configure PC13 as output (LED)
        let crh = core::ptr::read_volatile(GPIOC_CRH);
        core::ptr::write_volatile(GPIOC_CRH, (crh & !(0xF << 20)) | (0x3 << 20));

        // Initialize I2C1 (simplified)
        core::ptr::write_volatile(I2C1_CR1, 0x0000); // Disable I2C
        core::ptr::write_volatile(I2C1_CR2, 0x0008); // 8 MHz peripheral clock
        core::ptr::write_volatile(I2C1_CCR, 0x0028); // 100kHz I2C clock
        core::ptr::write_volatile(I2C1_TRISE, 0x0009); // Rise time
        core::ptr::write_volatile(I2C1_CR1, 0x0001); // Enable I2C
    }

    let mut led_state = false;

    loop {
        // Read temperature from TMP102
        let _temp = read_tmp102_temperature();

        // Toggle LED
        unsafe {
            if led_state {
                core::ptr::write_volatile(GPIOC_ODR, GPIO_ODR_ODR13);
            } else {
                core::ptr::write_volatile(GPIOC_ODR, 0);
            }
        }
        led_state = !led_state;

        // Delay
        for _ in 0..500_000 {
            cortex_m::asm::nop();
        }
    }
}

fn read_tmp102_temperature() -> i16 {
    unsafe {
        // Simplified I2C read (not full protocol)
        // In real hardware, this would:
        // 1. Send START
        // 2. Send device address + write
        // 3. Send register address
        // 4. Send repeated START
        // 5. Send device address + read
        // 6. Read 2 bytes
        // 7. Send STOP

        // For simulation, just read the data register
        // The emulator should return mock temperature data
        let temp_high = core::ptr::read_volatile(I2C1_DR) as u16;
        let temp_low = core::ptr::read_volatile(I2C1_DR) as u16;

        ((temp_high << 8) | temp_low) as i16
    }
}
