#![no_std]
#![no_main]

//! Bare-ELF P4 I2C0/BME280 smoke. It configures the board's J1 GPIO7/8
//! matrix route, reads the BME280 ID and raw temperature, then reports over
//! UART0. This does not exercise mask-ROM startup or physical UART wiring.

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;
use riscv_rt::entry;

const GPIO_BASE: usize = 0x500E_0000;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO_I2C_SCL_IN_SEL: *mut u32 = (GPIO_BASE + 0x268) as *mut u32;
const GPIO_I2C_SDA_IN_SEL: *mut u32 = (GPIO_BASE + 0x26C) as *mut u32;
const GPIO7_OUT_SEL: *mut u32 = (GPIO_BASE + 0x574) as *mut u32;
const GPIO8_OUT_SEL: *mut u32 = (GPIO_BASE + 0x578) as *mut u32;
const IO_MUX_GPIO7: *mut u32 = 0x500E_1020 as *mut u32;
const IO_MUX_GPIO8: *mut u32 = 0x500E_1024 as *mut u32;
const GPIO7_MASK: u32 = 1 << 7;
const GPIO8_MASK: u32 = 1 << 8;

const UART0_FIFO: *mut u32 = 0x500C_A000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x500C_A014 as *mut u32;

const I2C0_BASE: usize = 0x500C_4000;
const I2C_SCL_LOW_PERIOD: *mut u32 = I2C0_BASE as *mut u32;
const I2C_CTR: *mut u32 = (I2C0_BASE + 0x04) as *mut u32;
const I2C_FIFO_CONF: *mut u32 = (I2C0_BASE + 0x18) as *mut u32;
const I2C_DATA: *mut u32 = (I2C0_BASE + 0x1C) as *mut u32;
const I2C_INT_RAW: *const u32 = (I2C0_BASE + 0x20) as *const u32;
const I2C_INT_CLR: *mut u32 = (I2C0_BASE + 0x24) as *mut u32;
const I2C_SDA_HOLD: *mut u32 = (I2C0_BASE + 0x30) as *mut u32;
const I2C_SDA_SAMPLE: *mut u32 = (I2C0_BASE + 0x34) as *mut u32;
const I2C_SCL_HIGH_PERIOD: *mut u32 = (I2C0_BASE + 0x38) as *mut u32;
const I2C_SCL_START_HOLD: *mut u32 = (I2C0_BASE + 0x40) as *mut u32;
const I2C_SCL_RSTART_SETUP: *mut u32 = (I2C0_BASE + 0x44) as *mut u32;
const I2C_SCL_STOP_HOLD: *mut u32 = (I2C0_BASE + 0x48) as *mut u32;
const I2C_SCL_STOP_SETUP: *mut u32 = (I2C0_BASE + 0x4C) as *mut u32;
const I2C_CLK_CONF: *mut u32 = (I2C0_BASE + 0x54) as *mut u32;
const I2C_CMD0: *mut u32 = (I2C0_BASE + 0x58) as *mut u32;

const I2C_ADDRESS: u8 = 0x77;
const CMD_RSTART: u32 = 6;
const CMD_WRITE: u32 = 1;
const CMD_READ: u32 = 3;
const CMD_STOP: u32 = 2;
const CMD_ACK_VALUE: u32 = 1 << 10;
const FIFO_RX_RST: u32 = 1 << 12;
const FIFO_TX_RST: u32 = 1 << 13;
const INT_NACK: u32 = 1 << 10;
const INT_TRANS_COMPLETE: u32 = 1 << 7;
const CTR_TRANS_START: u32 = 1 << 5;
const SPIN_LIMIT: usize = 1_000_000;

#[entry]
fn main() -> ! {
    unsafe {
        // P4 UART0 APB clock is 90 MHz; 781 gives approximately 115200 baud.
        write_volatile(UART0_CLKDIV, 781);
        setup_i2c0_gpio_matrix();
        setup_i2c0();

        let mut chip_id = [0u8; 1];
        if !read_bme280_registers(0xD0, &mut chip_id) || chip_id[0] != 0x60 {
            uart_str(b"P4_BME280_FAIL CHIP_ID\n");
            wait_forever();
        }

        let mut temperature = [0u8; 3];
        if !read_bme280_registers(0xFA, &mut temperature) {
            uart_str(b"P4_BME280_FAIL TEMP_READ\n");
            wait_forever();
        }
        let raw_temperature = (u32::from(temperature[0]) << 12)
            | (u32::from(temperature[1]) << 4)
            | (u32::from(temperature[2]) >> 4);
        if raw_temperature != 0x082DC0 {
            uart_str(b"P4_BME280_FAIL TEMP\n");
            wait_forever();
        }

        uart_str(b"P4_BME280_ID=0x60_RAW_TEMP=0x082DC0_GPIO7_GPIO8\n");
        uart_str(b"P4X_C5_FUNCTION_EV_OK\n");
    }

    loop {
        core::hint::spin_loop();
    }
}

unsafe fn setup_i2c0_gpio_matrix() {
    // ESP-IDF P4 signal map: I2C0_SCL=68, I2C0_SDA=69. J1 pin 3 is GPIO7
    // (SDA); J1 pin 5 is GPIO8 (SCL). Input selectors use SIG_IN_SEL bit 7.
    for mux in [IO_MUX_GPIO7, IO_MUX_GPIO8] {
        let pad = read_volatile(mux);
        write_volatile(mux, (pad & !(0x7 << 12)) | (1 << 12) | (1 << 9));
    }
    write_volatile(GPIO7_OUT_SEL, 69 | (1 << 10)); // SDA output / GPIO enable
    write_volatile(GPIO8_OUT_SEL, 68 | (1 << 10)); // SCL output / GPIO enable
    write_volatile(GPIO_I2C_SCL_IN_SEL, (1 << 7) | 8);
    write_volatile(GPIO_I2C_SDA_IN_SEL, (1 << 7) | 7);
    write_volatile(GPIO_ENABLE_W1TS, GPIO7_MASK | GPIO8_MASK);
}

unsafe fn setup_i2c0() {
    // Select the 40 MHz XTAL source and 100 kHz SCL timing.
    write_volatile(I2C_CLK_CONF, 0);
    write_volatile(I2C_SCL_LOW_PERIOD, 199);
    write_volatile(I2C_SCL_HIGH_PERIOD, 199);
    write_volatile(I2C_SDA_HOLD, 2);
    write_volatile(I2C_SDA_SAMPLE, 2);
    write_volatile(I2C_SCL_START_HOLD, 10);
    write_volatile(I2C_SCL_RSTART_SETUP, 10);
    write_volatile(I2C_SCL_STOP_HOLD, 10);
    write_volatile(I2C_SCL_STOP_SETUP, 10);
}

unsafe fn read_bme280_registers(register: u8, destination: &mut [u8]) -> bool {
    if destination.is_empty() || destination.len() > 3 {
        return false;
    }

    write_volatile(I2C_FIFO_CONF, FIFO_RX_RST | FIFO_TX_RST);
    write_volatile(I2C_INT_CLR, u32::MAX);
    let command = |op: u32, count: u32| (op << 11) | count;
    write_volatile(I2C_CMD0, command(CMD_RSTART, 0));
    write_volatile(I2C_CMD0.add(1), command(CMD_WRITE, 2));
    write_volatile(I2C_CMD0.add(2), command(CMD_RSTART, 0));
    write_volatile(I2C_CMD0.add(3), command(CMD_WRITE, 1));
    write_volatile(
        I2C_CMD0.add(4),
        command(CMD_READ, destination.len() as u32) | CMD_ACK_VALUE,
    );
    write_volatile(I2C_CMD0.add(5), command(CMD_STOP, 0));

    write_volatile(I2C_DATA, u32::from(I2C_ADDRESS << 1));
    write_volatile(I2C_DATA, u32::from(register));
    write_volatile(I2C_DATA, u32::from((I2C_ADDRESS << 1) | 1));
    write_volatile(I2C_CTR, CTR_TRANS_START);

    let mut complete = false;
    for _ in 0..SPIN_LIMIT {
        if read_volatile(I2C_INT_RAW) & INT_TRANS_COMPLETE != 0 {
            complete = true;
            break;
        }
    }
    if !complete {
        return false;
    }

    let status = read_volatile(I2C_INT_RAW);
    write_volatile(I2C_INT_CLR, status);
    if status & INT_NACK != 0 {
        return false;
    }
    for byte in destination {
        *byte = read_volatile(I2C_DATA) as u8;
    }
    true
}

fn uart_str(bytes: &[u8]) {
    for &byte in bytes {
        unsafe { write_volatile(UART0_FIFO, u32::from(byte)) };
    }
}

fn wait_forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
