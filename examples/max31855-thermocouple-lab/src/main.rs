#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// STM32F103 register addresses
// RCC
const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;

// GPIO A (PA4 = CS, PA5 = SCK, PA6 = MISO, PA7 = MOSI)
const GPIOA_CRL: *mut u32 = 0x4001_0800 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4001_0810 as *mut u32;
const GPIOA_BRR:  *mut u32 = 0x4001_0814 as *mut u32;

// SPI1 (base 0x4001_3000)
const SPI1_CR1:  *mut u16 = 0x4001_3000 as *mut u16;
const SPI1_SR:   *const u16 = 0x4001_3008 as *const u16;
const SPI1_DR:   *mut u16 = 0x4001_300C as *mut u16;

// UART1 DR — for debug output
const UART1_DR: *mut u8 = (0x4001_3800 + 0x04) as *mut u8;

fn uart_byte(b: u8) {
    unsafe { core::ptr::write_volatile(UART1_DR, b) }
}

fn uart_str(s: &str) {
    for b in s.bytes() {
        uart_byte(b);
    }
}

fn uart_hex_u8(v: u8) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    uart_byte(HEX[(v >> 4) as usize]);
    uart_byte(HEX[(v & 0xF) as usize]);
}

fn uart_hex_u32(v: u32) {
    uart_hex_u8((v >> 24) as u8);
    uart_hex_u8((v >> 16) as u8);
    uart_hex_u8((v >> 8) as u8);
    uart_hex_u8(v as u8);
}

/// Transmit one byte via SPI1 and return the received byte.
fn spi_transfer(byte: u8) -> u8 {
    // Wait until TXE (bit 1) is set
    for _ in 0..1024 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        if sr & 0x0002 != 0 {
            break;
        }
    }
    unsafe { core::ptr::write_volatile(SPI1_DR, byte as u16) };
    // Wait until RXNE (bit 0) is set
    for _ in 0..1024 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        if sr & 0x0001 != 0 {
            break;
        }
    }
    let rx = unsafe { core::ptr::read_volatile(SPI1_DR) };
    rx as u8
}

/// Read the full 32-bit status word from MAX31855 via 4-byte SPI transfer.
fn max31855_read() -> u32 {
    // CS low (PA4 = bit 4, BRR clears)
    unsafe { core::ptr::write_volatile(GPIOA_BRR, 1 << 4) };

    let b0 = spi_transfer(0x00) as u32;
    let b1 = spi_transfer(0x00) as u32;
    let b2 = spi_transfer(0x00) as u32;
    let b3 = spi_transfer(0x00) as u32;

    // CS high (PA4, BSRR sets)
    unsafe { core::ptr::write_volatile(GPIOA_BSRR, 1 << 4) };

    (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable RCC for SPI1 (APB2ENR bit 12) and GPIOA (bit 2)
        let apb2enr = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2enr | (1 << 12) | (1 << 2));

        // Configure GPIOA:
        //   PA4  = output push-pull 50 MHz (CS)  → CRL bits [19:16] = 0b0011
        //   PA5  = AF push-pull 50 MHz (SCK)     → CRL bits [23:20] = 0b1011
        //   PA6  = input floating (MISO)          → CRL bits [27:24] = 0b0100
        //   PA7  = AF push-pull 50 MHz (MOSI)     → CRL bits [31:28] = 0b1011
        let mut crl = core::ptr::read_volatile(GPIOA_CRL);
        // Clear PA4..PA7 nibbles (bits 16..31)
        crl &= 0x0000_FFFF;
        // PA4: output PP 50 MHz = 0011
        // PA5: AF PP 50 MHz = 1011
        // PA6: input floating = 0100
        // PA7: AF PP 50 MHz = 1011
        crl |= 0xB4B3_0000;
        core::ptr::write_volatile(GPIOA_CRL, crl);

        // CS high initially
        core::ptr::write_volatile(GPIOA_BSRR, 1 << 4);

        // Configure SPI1:
        //   Master mode (bit 2), BR=000 (f/2), CPOL=0, CPHA=0, SPE (bit 6)
        //   CR1 = SPE(6) | MSTR(2) | BR=000 = 0x0044
        core::ptr::write_volatile(SPI1_CR1, 0x0044u16);
    }

    uart_str("MAX31855 Thermocouple Lab\n");

    loop {
        let word = max31855_read();

        // Decode thermocouple temperature: bits [31:18], 14-bit signed, unit = 0.25 °C
        let tc_raw = (word >> 18) & 0x3FFF;
        // Sign-extend 14-bit value
        let tc_signed: i32 = if tc_raw & 0x2000 != 0 {
            (tc_raw as i32) | (!0x3FFFi32)
        } else {
            tc_raw as i32
        };
        let tc_x4 = tc_signed; // already ×4 (0.25°C resolution)

        // Decode internal temperature: bits [15:4], 12-bit signed, unit = 0.0625 °C
        let int_raw = (word >> 4) & 0x0FFF;
        let int_signed: i32 = if int_raw & 0x0800 != 0 {
            (int_raw as i32) | (!0x0FFFi32)
        } else {
            int_raw as i32
        };

        let fault = (word >> 16) & 1;

        // Print: "word=0xXXXXXXXX TC=NNN/4 INT=NNN/16 FAULT=N\n"
        uart_str("word=0x");
        uart_hex_u32(word);
        uart_str(" TC_q4=");
        uart_hex_u32(tc_x4 as u32);
        uart_str(" INT_q12=");
        uart_hex_u32(int_signed as u32);
        uart_str(" FAULT=");
        uart_byte(b'0' + fault as u8);
        uart_byte(b'\n');

        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
