#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

const GPIO_BASE: usize = 0x6009_1000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_IN: *const u32 = (GPIO_BASE + 0x3C) as *const u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO_FUNC45_IN_SEL: *mut u32 = (GPIO_BASE + 0x154 + 45 * 4) as *mut u32;
const GPIO_FUNC46_IN_SEL: *mut u32 = (GPIO_BASE + 0x154 + 46 * 4) as *mut u32;
const GPIO4_OUT_SEL: *mut u32 = (GPIO_BASE + 0x554 + 4 * 4) as *mut u32;
const GPIO5_OUT_SEL: *mut u32 = (GPIO_BASE + 0x554 + 5 * 4) as *mut u32;
const GPIO10_OUT_SEL: *mut u32 = (GPIO_BASE + 0x554 + 10 * 4) as *mut u32;
const GPIO4_MASK: u32 = 1 << 4;
const GPIO5_MASK: u32 = 1 << 5;
const GPIO10_MASK: u32 = 1 << 10;

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

#[entry]
fn main() -> ! {
    unsafe {
        // GPIO4/5 are exposed on J1 pins 9/10. The H2 GPIO matrix maps I2C0
        // SCL/SDA to signals 45/46; GPIO10 remains a separate output probe.
        write_volatile(GPIO4_OUT_SEL, 46 | (1 << 9)); // SDA, GPIO_ENABLE OEN
        write_volatile(GPIO5_OUT_SEL, 45 | (1 << 9)); // SCL, GPIO_ENABLE OEN
        write_volatile(GPIO_FUNC45_IN_SEL, (1 << 7) | 5); // SCL input from GPIO5
        write_volatile(GPIO_FUNC46_IN_SEL, (1 << 7) | 4); // SDA input from GPIO4
        write_volatile(GPIO10_OUT_SEL, 128); // GPIO_OUT latch
        write_volatile(GPIO_OUT_W1TC, GPIO10_MASK);
        write_volatile(GPIO_ENABLE_W1TS, GPIO4_MASK | GPIO5_MASK | GPIO10_MASK);
        if read_volatile(GPIO4_OUT_SEL) != 46 | (1 << 9)
            || read_volatile(GPIO5_OUT_SEL) != 45 | (1 << 9)
            || read_volatile(GPIO_FUNC45_IN_SEL) != (1 << 7) | 5
            || read_volatile(GPIO_FUNC46_IN_SEL) != (1 << 7) | 4
        {
            uart_str(b"ESP32H2_I2C_MATRIX_CONFIG_FAIL\n");
            wait_forever();
        }
        if read_volatile(GPIO_IN) & (GPIO4_MASK | GPIO5_MASK) != GPIO4_MASK | GPIO5_MASK {
            uart_str(b"ESP32H2_I2C_IDLE_LINE_FAIL\n");
            wait_forever();
        }

        setup_i2c0();

        let mut chip_id = [0u8; 1];
        if !read_bme280_registers(0xD0, &mut chip_id) {
            uart_str(b"ESP32H2_BME280_FAIL CHIP_ID_READ\n");
            wait_forever();
        }
        if chip_id[0] != 0x60 {
            uart_str(b"ESP32H2_BME280_FAIL CHIP_ID_VALUE\n");
            wait_forever();
        }

        let mut temperature = [0u8; 3];
        if !read_bme280_registers(0xFA, &mut temperature) {
            uart_str(b"ESP32H2_BME280_FAIL TEMP_READ\n");
            wait_forever();
        }
        let raw_temperature = (u32::from(temperature[0]) << 12)
            | (u32::from(temperature[1]) << 4)
            | (u32::from(temperature[2]) >> 4);
        if raw_temperature != 0x082DC0 {
            uart_str(b"ESP32H2_BME280_FAIL TEMP\n");
            wait_forever();
        }
        // Keep each UART burst short enough for the H2 model's polled TX FIFO.
        // Both markers occur only after the sensor bytes have been checked.
        uart_str(b"ESP32H2_BME280_OK\n");

        // H2 UART0's profile divider is 694; bytes reach the simulator console.
        write_volatile(UART0_CLKDIV, 694);
        for _ in 0..4 {
            write_volatile(GPIO_OUT_W1TS, GPIO10_MASK);
            delay();
            write_volatile(GPIO_OUT_W1TC, GPIO10_MASK);
            delay();
        }
        uart_str(b"ESP32H2_UART0_OK\n");
    }

    loop {}
}

unsafe fn setup_i2c0() {
    // Select the 40 MHz XTAL source and program a 100 kHz I2C SCL.
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
    write_volatile(I2C_CMD0.add(4), command(CMD_READ, destination.len() as u32) | CMD_ACK_VALUE);
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
        uart_str(b"ESP32H2_I2C_TIMEOUT\n");
        return false;
    }

    let status = read_volatile(I2C_INT_RAW);
    write_volatile(I2C_INT_CLR, status);
    if status & INT_NACK != 0 {
        uart_str(b"ESP32H2_I2C_NACK\n");
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
