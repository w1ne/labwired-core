#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const RESETS_BASE: u32 = 0x4000_C000;
const RESETS_RESET: *mut u32 = RESETS_BASE as *mut u32;
const RESETS_RESET_DONE: *const u32 = (RESETS_BASE + 0x08) as *const u32;
const RESETS_IO_BANK0: u32 = 1 << 5;
const RESETS_PADS_BANK0: u32 = 1 << 8;
const RESETS_I2C0: u32 = 1 << 3;
const RESETS_UART0: u32 = 1 << 22;
const PERIPHERAL_RESETS: u32 = RESETS_IO_BANK0 | RESETS_PADS_BANK0 | RESETS_I2C0 | RESETS_UART0;

const CLK_PERI_CTRL: *mut u32 = 0x4000_8048 as *mut u32;
const CLK_PERI_ENABLE: u32 = 1 << 11;

const UART0_BASE: u32 = 0x4003_4000;
const UART0_DR: *mut u32 = UART0_BASE as *mut u32;
const UART0_FR: *const u32 = (UART0_BASE + 0x18) as *const u32;
const UART0_IBRD: *mut u32 = (UART0_BASE + 0x24) as *mut u32;
const UART0_FBRD: *mut u32 = (UART0_BASE + 0x28) as *mut u32;
const UART0_LCR_H: *mut u32 = (UART0_BASE + 0x2C) as *mut u32;
const UART0_CR: *mut u32 = (UART0_BASE + 0x30) as *mut u32;
const UART0_TXFF: u32 = 1 << 5;

const IO_BANK0_BASE: u32 = 0x4001_4000;
const GPIO0_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04) as *mut u32;
const GPIO4_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04 + 4 * 8) as *mut u32;
const GPIO5_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04 + 5 * 8) as *mut u32;

const I2C0_BASE: u32 = 0x4004_4000;
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

unsafe fn release_peripherals_from_reset() -> bool {
    let reset = read(RESETS_RESET);
    write(RESETS_RESET, reset & !PERIPHERAL_RESETS);

    for _ in 0..SPIN_LIMIT {
        if read(RESETS_RESET_DONE) & PERIPHERAL_RESETS == PERIPHERAL_RESETS {
            return true;
        }
    }
    false
}

unsafe fn setup_uart0() {
    // The board has no USB CDC bridge. UART0 on GP0 is diagnostic output for
    // the simulator; on hardware it is available on the Pico header.
    write(GPIO0_CTRL, 2); // GP0 FUNCSEL 2 = UART0 TX.
    write(CLK_PERI_CTRL, CLK_PERI_ENABLE); // AUXSRC reset default is clk_sys.
    write(UART0_CR, 0);
    // clk_peri is 125 MHz in the Pico default system-clock configuration.
    // The UART sink captures bytes independently of baud timing in the twin.
    write(UART0_IBRD, 67);
    write(UART0_FBRD, 52);
    write(UART0_LCR_H, (0b11 << 5) | (1 << 4)); // 8N1, FIFO enabled.
    write(UART0_CR, (1 << 0) | (1 << 8)); // UARTEN | TXE.
}

fn uart_byte(value: u8) {
    unsafe {
        for _ in 0..SPIN_LIMIT {
            if read(UART0_FR) & UART0_TXFF == 0 {
                break;
            }
        }
        write(UART0_DR, u32::from(value));
    }
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
    // GP4/GP5 FUNCSEL 3 routes I2C0 SDA/SCL to the Pico header pins.
    write(GPIO4_CTRL, 3);
    write(GPIO5_CTRL, 3);

    // DW_apb_i2c master, standard speed, repeated-start enabled, slave disabled.
    write(IC_ENABLE, 0);
    write(IC_CON, (1 << 0) | (1 << 1) | (1 << 5) | (1 << 6) | (1 << 8));
    write(IC_SS_SCL_HCNT, 625);
    write(IC_SS_SCL_LCNT, 625);
    write(IC_TAR, BME280_ADDRESS);
    write(IC_ENABLE, 1);
}

unsafe fn read_register(register: u8) -> Result<u8, u32> {
    write(IC_DATA_CMD, u32::from(register));
    let mut tx_empty = false;
    for _ in 0..SPIN_LIMIT {
        let status = read(IC_RAW_INTR_STAT);
        if status & INTR_TX_ABRT != 0 {
            let reason = read(IC_TX_ABRT_SOURCE);
            let _ = read(IC_CLR_TX_ABRT);
            return Err(reason);
        }
        if status & INTR_TX_EMPTY != 0 {
            tx_empty = true;
            break;
        }
    }
    if !tx_empty {
        return Err(0xFFFF_FFFE);
    }

    // Repeated-start preserves the register pointer for the one-byte read.
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
    Err(u32::MAX)
}

#[entry]
fn main() -> ! {
    unsafe {
        let resets_released = release_peripherals_from_reset();
        setup_uart0();
        if !resets_released {
            uart_str(b"RP2040_RESET_TIMEOUT\n");
            loop {}
        }
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

        uart_str(b"PICOW_BME280_OK ID=0x60 TEMP_RAW=0x");
        uart_hex20(raw_temperature);
        uart_str(b" I2C0=GP4/GP5\n");
    }

    loop {}
}
