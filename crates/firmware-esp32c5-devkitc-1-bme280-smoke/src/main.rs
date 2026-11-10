#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

const GPIO_BASE: usize = 0x6009_1000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x38) as *mut u32;
const GPIO_I2C_SCL_IN_SEL: *mut u32 = (GPIO_BASE + 0x2D4 + 46 * 4) as *mut u32;
const GPIO_I2C_SDA_IN_SEL: *mut u32 = (GPIO_BASE + 0x2D4 + 47 * 4) as *mut u32;
const GPIO8_OUT_SEL: *mut u32 = (GPIO_BASE + 0xAD4 + 8 * 4) as *mut u32;
const GPIO9_OUT_SEL: *mut u32 = (GPIO_BASE + 0xAD4 + 9 * 4) as *mut u32;

const IO_MUX_GPIO5: *mut u32 = 0x6009_0014 as *mut u32;
const IO_MUX_GPIO8: *mut u32 = 0x6009_0020 as *mut u32;
const IO_MUX_GPIO9: *mut u32 = 0x6009_0024 as *mut u32;
const GPIO5_MASK: u32 = 1 << 5;
const GPIO8_MASK: u32 = 1 << 8;
const GPIO9_MASK: u32 = 1 << 9;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

const I2C0_BASE: usize = 0x6000_4000;
const I2C_SCL_LOW_PERIOD: *mut u32 = I2C0_BASE as *mut u32;
const I2C_CTR: *mut u32 = (I2C0_BASE + 0x04) as *mut u32;
const I2C_FIFO_CONF: *mut u32 = (I2C0_BASE + 0x18) as *mut u32;
const I2C_DATA: *mut u32 = (I2C0_BASE + 0x1C) as *mut u32;
const I2C_INT_RAW: *const u32 = (I2C0_BASE + 0x20) as *const u32;
const I2C_INT_CLR: *mut u32 = (I2C0_BASE + 0x24) as *mut u32;
const I2C_CMD0: *mut u32 = (I2C0_BASE + 0x58) as *mut u32;
const I2C_SCL_HIGH_PERIOD: *mut u32 = (I2C0_BASE + 0x38) as *mut u32;
const I2C_SDA_HOLD: *mut u32 = (I2C0_BASE + 0x30) as *mut u32;
const I2C_SDA_SAMPLE: *mut u32 = (I2C0_BASE + 0x34) as *mut u32;
const I2C_SCL_START_HOLD: *mut u32 = (I2C0_BASE + 0x40) as *mut u32;
const I2C_SCL_RSTART_SETUP: *mut u32 = (I2C0_BASE + 0x44) as *mut u32;
const I2C_SCL_STOP_HOLD: *mut u32 = (I2C0_BASE + 0x48) as *mut u32;
const I2C_SCL_STOP_SETUP: *mut u32 = (I2C0_BASE + 0x4C) as *mut u32;
const I2C_CLK_CONF: *mut u32 = (I2C0_BASE + 0x54) as *mut u32;

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
const SPIN_LIMIT: usize = 120_000;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        write_volatile(UART0_CLKDIV, 694);

        // GPIO5 remains a direct-output indicator for the board's J3 pin 9.
        select_gpio_function(IO_MUX_GPIO5);
        write_volatile(GPIO_OUT_W1TC, GPIO5_MASK);
        write_volatile(GPIO_ENABLE_W1TS, GPIO5_MASK);

        setup_i2c0_gpio_matrix();
        setup_i2c0();

        let mut chip_id = [0u8; 1];
        if !read_bme280_registers(0xD0, &mut chip_id) || chip_id[0] != 0x60 {
            uart_str(b"C5_BME280_FAIL CHIP_ID\n");
            wait_forever();
        }

        let mut temperature = [0u8; 3];
        if !read_bme280_registers(0xFA, &mut temperature) {
            uart_str(b"C5_BME280_FAIL TEMP_READ\n");
            wait_forever();
        }

        let raw_temperature = (u32::from(temperature[0]) << 12)
            | (u32::from(temperature[1]) << 4)
            | (u32::from(temperature[2]) >> 4);
        if raw_temperature != 0x082DC0 {
            uart_str(b"C5_BME280_FAIL TEMP\n");
            wait_forever();
        }
        // Keep the assertion within a short UART burst; the exact ID and raw
        // value are checked above before this success marker is emitted.
        uart_str(b"C5_BME280_OK\n");

        loop {
            write_volatile(GPIO_OUT_W1TS, GPIO5_MASK);
            delay();
            write_volatile(GPIO_OUT_W1TC, GPIO5_MASK);
            delay();
        }
    }
}

unsafe fn select_gpio_function(mux: *mut u32) {
    let pad = read_volatile(mux);
    write_volatile(mux, (pad & !(0x7 << 12)) | (1 << 12));
}

unsafe fn setup_i2c0_gpio_matrix() {
    // ESP-IDF C5 signal map: I2CEXT0_SCL=46 and SDA=47 for both directions.
    // C5 input selectors start at GPIO+0x2D4 with GPIO_SEL[6:0] and SIG_IN_SEL[8].
    // Output selectors start at GPIO+0xAD4; OEN_SEL[10] chooses GPIO_ENABLE.
    // GPIO7 is a board strapping pin, so the breakout uses GPIO8/9 instead.
    select_gpio_function(IO_MUX_GPIO8);
    select_gpio_function(IO_MUX_GPIO9);
    write_volatile(GPIO8_OUT_SEL, 47 | (1 << 10)); // SDA
    write_volatile(GPIO9_OUT_SEL, 46 | (1 << 10)); // SCL
    write_volatile(GPIO_I2C_SCL_IN_SEL, (1 << 8) | 9);
    write_volatile(GPIO_I2C_SDA_IN_SEL, (1 << 8) | 8);
    write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK | GPIO9_MASK);
}

unsafe fn setup_i2c0() {
    // Select the 40 MHz XTAL clock, then set 200 low + 200 high module ticks:
    // 100 kHz I2C, or 2,400 ESP32-C5 core cycles per SCL period at 240 MHz.
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
        let status = read_volatile(I2C_INT_RAW);
        if status & INT_TRANS_COMPLETE != 0 {
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

fn uart_byte(byte: u8) {
    unsafe { write_volatile(UART0_FIFO, u32::from(byte)) };
}

fn uart_str(bytes: &[u8]) {
    for &byte in bytes {
        uart_byte(byte);
    }
}

#[inline(never)]
fn delay() {
    for _ in 0..1_000 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

fn wait_forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    wait_forever()
}
