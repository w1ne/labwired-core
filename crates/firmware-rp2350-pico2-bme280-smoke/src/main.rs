#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const UART0_DR: *mut u32 = 0x4007_0000 as *mut u32;
const IO_BANK0_BASE: u32 = 0x4002_8000;
const GPIO0_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04) as *mut u32;
const GPIO4_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04 + 4 * 8) as *mut u32;
const GPIO5_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04 + 5 * 8) as *mut u32;
const GPIO25_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04 + 25 * 8) as *mut u32;

const SIO_BASE: u32 = 0xD000_0000;
const SIO_GPIO_OUT_SET: *mut u32 = (SIO_BASE + 0x18) as *mut u32;
const SIO_GPIO_OUT_CLR: *mut u32 = (SIO_BASE + 0x20) as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = (SIO_BASE + 0x38) as *mut u32;
const LED_MASK: u32 = 1 << 25;

const I2C0_BASE: u32 = 0x4009_0000;
const IC_CON: *mut u32 = I2C0_BASE as *mut u32;
const IC_TAR: *mut u32 = (I2C0_BASE + 0x04) as *mut u32;
const IC_DATA_CMD: *mut u32 = (I2C0_BASE + 0x10) as *mut u32;
const IC_SS_SCL_HCNT: *mut u32 = (I2C0_BASE + 0x14) as *mut u32;
const IC_SS_SCL_LCNT: *mut u32 = (I2C0_BASE + 0x18) as *mut u32;
const IC_RAW_INTR_STAT: *const u32 = (I2C0_BASE + 0x34) as *const u32;
const IC_CLR_TX_ABRT: *const u32 = (I2C0_BASE + 0x54) as *const u32;
const IC_CLR_STOP_DET: *const u32 = (I2C0_BASE + 0x60) as *const u32;
const IC_ENABLE: *mut u32 = (I2C0_BASE + 0x6C) as *mut u32;
const IC_TX_ABRT_SOURCE: *const u32 = (I2C0_BASE + 0x80) as *const u32;

const BME280_ADDRESS: u32 = 0x77;
const INTR_TX_EMPTY: u32 = 1 << 4;
const INTR_TX_ABRT: u32 = 1 << 6;
const INTR_STOP_DET: u32 = 1 << 9;
const CMD_READ: u32 = 1 << 8;
const CMD_STOP: u32 = 1 << 9;
const CMD_RESTART: u32 = 1 << 10;
const SPIN_LIMIT: usize = 100_000;

#[inline]
unsafe fn write(reg: *mut u32, value: u32) {
    core::ptr::write_volatile(reg, value);
}

#[inline]
unsafe fn read(reg: *const u32) -> u32 {
    core::ptr::read_volatile(reg)
}

fn uart_byte(value: u8) {
    unsafe { write(UART0_DR, u32::from(value)) }
}

fn uart_str(value: &[u8]) {
    for &byte in value {
        uart_byte(byte);
    }
}

fn uart_hex_byte(value: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    uart_byte(HEX[(value >> 4) as usize]);
    uart_byte(HEX[(value & 0x0F) as usize]);
}

fn uart_hex20(value: u32) {
    uart_hex_byte((value >> 16) as u8);
    uart_hex_byte((value >> 8) as u8);
    uart_hex_byte(value as u8);
}

unsafe fn setup_i2c0() {
    // RP2350 IO_BANK0 function select 3 routes GP4/GP5 to I2C0 SDA/SCL.
    write(GPIO4_CTRL, 3);
    write(GPIO5_CTRL, 3);

    // DW_apb_i2c master, standard speed, repeated-start enabled, slave disabled.
    write(IC_ENABLE, 0);
    write(IC_CON, (1 << 0) | (1 << 1) | (1 << 5) | (1 << 6) | (1 << 8));
    write(IC_SS_SCL_HCNT, 750);
    write(IC_SS_SCL_LCNT, 750);
    write(IC_TAR, BME280_ADDRESS);
    write(IC_ENABLE, 1);
}

unsafe fn setup_led() {
    // The Pico 2's onboard LED is active-high on GP25; FUNCSEL 5 selects SIO.
    write(GPIO25_CTRL, 5);
    write(SIO_GPIO_OUT_CLR, LED_MASK);
    write(SIO_GPIO_OE_SET, LED_MASK);
}

unsafe fn signal_sensor_read() {
    // Four visible transitions make the successful sensor transaction
    // observable on the actual Pico 2 LED and in the simulator GPIO oracle.
    for _ in 0..2 {
        write(SIO_GPIO_OUT_SET, LED_MASK);
        for _ in 0..2_000 {
            core::hint::spin_loop();
        }
        write(SIO_GPIO_OUT_CLR, LED_MASK);
        for _ in 0..2_000 {
            core::hint::spin_loop();
        }
    }
}

unsafe fn read_register(register: u8) -> Result<u8, u32> {
    write(IC_DATA_CMD, u32::from(register));
    for _ in 0..SPIN_LIMIT {
        let status = read(IC_RAW_INTR_STAT);
        if status & INTR_TX_ABRT != 0 {
            let reason = read(IC_TX_ABRT_SOURCE);
            let _ = read(IC_CLR_TX_ABRT);
            return Err(reason);
        }
        if status & INTR_TX_EMPTY != 0 {
            break;
        }
    }

    // The register pointer remains active across this repeated-start read.
    write(IC_DATA_CMD, CMD_READ | CMD_RESTART | CMD_STOP);
    for _ in 0..SPIN_LIMIT {
        let status = read(IC_RAW_INTR_STAT);
        if status & INTR_TX_ABRT != 0 {
            let reason = read(IC_TX_ABRT_SOURCE);
            let _ = read(IC_CLR_TX_ABRT);
            return Err(reason);
        }
        if status & INTR_STOP_DET != 0 {
            let value = read(IC_DATA_CMD) as u8;
            let _ = read(IC_CLR_STOP_DET);
            return Ok(value);
        }
    }
    Err(0xFFFF_FFFF)
}

#[entry]
fn main() -> ! {
    unsafe {
        // UART0 TX on GP0 is simulator diagnostic output; I2C0 uses GP4/GP5.
        write(GPIO0_CTRL, 2);
        setup_led();
        setup_i2c0();

        match read_register(0xD0) {
            Ok(0x60) => uart_str(b"BME280_ID=0x60\n"),
            Ok(id) => {
                uart_str(b"BME280_ID_ERROR=0x");
                uart_hex_byte(id);
                uart_str(b"\n");
                loop {}
            }
            Err(reason) => {
                uart_str(b"BME280_I2C_ERROR=0x");
                uart_hex_byte((reason >> 8) as u8);
                uart_hex_byte(reason as u8);
                uart_str(b"\n");
                loop {}
            }
        }

        let msb = match read_register(0xFA) {
            Ok(value) => value,
            Err(_) => {
                uart_str(b"BME280_TEMP_READ_ERROR\n");
                loop {}
            }
        };
        let lsb = match read_register(0xFB) {
            Ok(value) => value,
            Err(_) => {
                uart_str(b"BME280_TEMP_READ_ERROR\n");
                loop {}
            }
        };
        let xlsb = match read_register(0xFC) {
            Ok(value) => value,
            Err(_) => {
                uart_str(b"BME280_TEMP_READ_ERROR\n");
                loop {}
            }
        };
        let raw_temperature = (u32::from(msb) << 12) | (u32::from(lsb) << 4) | u32::from(xlsb >> 4);

        uart_str(b"PICO2_BME280_OK ID=0x60 TEMP_RAW=0x");
        uart_hex20(raw_temperature);
        uart_str(b" I2C0=GP4/GP5\n");
        signal_sensor_read();
    }

    loop {}
}
