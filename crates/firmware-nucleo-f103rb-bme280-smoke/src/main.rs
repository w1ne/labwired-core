// NUCLEO-F103RB + BME280 smoke on the reset HSI clock (8 MHz).
// I2C1 uses default PB6/PB7; USART2 is routed to the onboard ST-LINK VCP.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_APB1ENR: *mut u32 = 0x4002_101C as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;

const GPIOA_CRL: *mut u32 = 0x4001_0800 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4001_0810 as *mut u32;
const GPIOB_CRH: *mut u32 = 0x4001_0C04 as *mut u32;

const I2C1_CR1: *mut u32 = 0x4000_5400 as *mut u32;
const I2C1_CR2: *mut u32 = 0x4000_5404 as *mut u32;
const I2C1_DR: *mut u32 = 0x4000_5410 as *mut u32;
const I2C1_SR1: *const u32 = 0x4000_5414 as *const u32;
const I2C1_SR2: *const u32 = 0x4000_5418 as *const u32;
const I2C1_CCR: *mut u32 = 0x4000_541C as *mut u32;
const I2C1_TRISE: *mut u32 = 0x4000_5420 as *mut u32;

const USART2_SR: *const u32 = 0x4000_4400 as *const u32;
const USART2_DR: *mut u32 = 0x4000_4404 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_4408 as *mut u32;
const USART2_CR1: *mut u32 = 0x4000_440C as *mut u32;

const I2C1EN: u32 = 1 << 21;
const GPIOAEN: u32 = 1 << 2;
const GPIOBEN: u32 = 1 << 3;
const USART2EN: u32 = 1 << 17;

const CR1_PE: u32 = 1 << 0;
const CR1_START: u32 = 1 << 8;
const CR1_STOP: u32 = 1 << 9;
const CR1_ACK: u32 = 1 << 10;
const SR1_SB: u32 = 1 << 0;
const SR1_ADDR: u32 = 1 << 1;
const SR1_BTF: u32 = 1 << 2;
const SR1_RXNE: u32 = 1 << 6;
const SR1_TXE: u32 = 1 << 7;
const SR1_AF: u32 = 1 << 10;
const USART2_TXE: u32 = 1 << 7;

const BME280_ADDRESS: u8 = 0x77;
const POLL_LIMIT: u32 = 20_000;

#[entry]
fn main() -> ! {
    unsafe { board_init() };
    send(b"NUCLEO_F103RB_BME280_BOOT\r\n");

    match read_bme280_register(0xD0) {
        Some(0x60) => send(b"NUCLEO_F103RB_BME280_ID=0x60\r\n"),
        Some(id) => {
            send(b"NUCLEO_F103RB_BME280_ID=0x");
            send_hex_byte(id);
            send(b"\r\n");
        }
        None => send(b"NUCLEO_F103RB_BME280_I2C_ERROR\r\n"),
    }

    loop {
        led_set(true);
        delay(20_000);
        led_set(false);
        delay(20_000);
    }
}

unsafe fn board_init() {
    // Keep the board on the reset 8 MHz HSI clock for deterministic bus timing.
    write_volatile(RCC_APB2ENR, read_volatile(RCC_APB2ENR) | GPIOAEN | GPIOBEN);
    write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | I2C1EN | USART2EN);

    // PA2 = USART2_TX (AF push-pull), PA3 = USART2_RX (floating input),
    // PA5 = LD2 (push-pull output, active high).
    let mut gpioa_crl = read_volatile(GPIOA_CRL);
    gpioa_crl = (gpioa_crl & !((0xF << 8) | (0xF << 12) | (0xF << 20)))
        | (0xB << 8)
        | (0x4 << 12)
        | (0x2 << 20);
    write_volatile(GPIOA_CRL, gpioa_crl);
    write_volatile(GPIOA_BSRR, 1 << (5 + 16)); // LD2 off

    // Default I2C1 pads PB6/SCL and PB7/SDA, AF open-drain at 50 MHz.
    let mut gpiob_crh = read_volatile(GPIOB_CRH);
    gpiob_crh = (gpiob_crh & !((0xF << 24) | (0xF << 28))) | (0xF << 24) | (0xF << 28);
    write_volatile(GPIOB_CRH, gpiob_crh);

    // USART2 on PA2 at 115200 baud from the reset 8 MHz PCLK1.
    write_volatile(USART2_BRR, 0x45);
    write_volatile(USART2_CR1, (1 << 13) | (1 << 3)); // UE | TE

    // Standard-mode I2C1 at 100 kHz from the reset 8 MHz PCLK1.
    write_volatile(I2C1_CR1, 0);
    write_volatile(I2C1_CR2, 8);
    write_volatile(I2C1_CCR, 40);
    write_volatile(I2C1_TRISE, 9);
    write_volatile(I2C1_CR1, CR1_PE | CR1_ACK);
}

fn read_bme280_register(register: u8) -> Option<u8> {
    unsafe {
        // Write the register pointer, issue a repeated START, then read one byte.
        write_volatile(I2C1_CR1, read_volatile(I2C1_CR1) | CR1_START);
        if !wait_for_sr1(SR1_SB) {
            return None;
        }
        write_volatile(I2C1_DR, (BME280_ADDRESS as u32) << 1);
        if !wait_for_address() {
            return None;
        }
        let _ = read_volatile(I2C1_SR2);

        if !wait_for_sr1(SR1_TXE) {
            return None;
        }
        write_volatile(I2C1_DR, register as u32);
        if !wait_for_sr1(SR1_BTF) {
            return None;
        }

        write_volatile(I2C1_CR1, read_volatile(I2C1_CR1) | CR1_START);
        if !wait_for_sr1(SR1_SB) {
            return None;
        }
        write_volatile(I2C1_DR, ((BME280_ADDRESS as u32) << 1) | 1);
        if !wait_for_address() {
            return None;
        }

        write_volatile(I2C1_CR1, read_volatile(I2C1_CR1) & !CR1_ACK);
        let _ = read_volatile(I2C1_SR2);
        write_volatile(I2C1_CR1, read_volatile(I2C1_CR1) | CR1_STOP);
        if !wait_for_sr1(SR1_RXNE) {
            return None;
        }
        let value = read_volatile(I2C1_DR) as u8;
        write_volatile(I2C1_CR1, read_volatile(I2C1_CR1) | CR1_ACK);
        Some(value)
    }
}

fn wait_for_address() -> bool {
    for _ in 0..POLL_LIMIT {
        let status = unsafe { read_volatile(I2C1_SR1) };
        if status & SR1_ADDR != 0 {
            return true;
        }
        if status & SR1_AF != 0 {
            return false;
        }
    }
    false
}

fn wait_for_sr1(mask: u32) -> bool {
    for _ in 0..POLL_LIMIT {
        if unsafe { read_volatile(I2C1_SR1) } & mask != 0 {
            return true;
        }
    }
    false
}

fn led_set(on: bool) {
    let bits = if on { 1 << 5 } else { 1 << (5 + 16) };
    unsafe { write_volatile(GPIOA_BSRR, bits) };
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        let mut remaining = POLL_LIMIT;
        while unsafe { read_volatile(USART2_SR) } & USART2_TXE == 0 && remaining > 0 {
            remaining -= 1;
        }
        if remaining == 0 {
            return;
        }
        unsafe { write_volatile(USART2_DR, *byte as u32) };
    }
}

fn send_hex_byte(value: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    send(&[HEX[(value >> 4) as usize], HEX[(value & 0x0F) as usize]]);
}

fn delay(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}
