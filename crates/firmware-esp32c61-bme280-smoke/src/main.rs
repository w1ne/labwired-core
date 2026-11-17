#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const PCR_BASE: usize = 0x6009_6000;
const PCR_UART0_CONF: *mut u32 = PCR_BASE as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = (PCR_BASE + 0x04) as *mut u32;
const PCR_I2C_CONF: *mut u32 = (PCR_BASE + 0x2C) as *mut u32;
const PCR_I2C_SCLK_CONF: *mut u32 = (PCR_BASE + 0x30) as *mut u32;

const UART0_BASE: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

const GPIO_BASE: usize = 0x6009_1000;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x38) as *mut u32;
const GPIO_I2C_SCL_IN_SEL: *mut u32 = (GPIO_BASE + 0x2D4 + 46 * 4) as *mut u32;
const GPIO_I2C_SDA_IN_SEL: *mut u32 = (GPIO_BASE + 0x2D4 + 47 * 4) as *mut u32;
const GPIO6_OUT_SEL: *mut u32 = (GPIO_BASE + 0xAD4 + 6 * 4) as *mut u32;
const GPIO22_OUT_SEL: *mut u32 = (GPIO_BASE + 0xAD4 + 22 * 4) as *mut u32;
const GPIO6_MASK: u32 = 1 << 6;
const GPIO22_MASK: u32 = 1 << 22;

const IO_MUX_GPIO6: *mut u32 = 0x6009_001C as *mut u32;
const IO_MUX_GPIO22: *mut u32 = 0x6009_005C as *mut u32;

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
        // Enable UART0 and I2C APB/function clocks. C61 places I2C PCR
        // registers at +0x2C/+0x30, unlike the C6 PCR layout.
        let uart_conf = core::ptr::read_volatile(PCR_UART0_CONF);
        core::ptr::write_volatile(PCR_UART0_CONF, uart_conf | 1);
        let uart_sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, uart_sclk | (1 << 22));
        let i2c_conf = core::ptr::read_volatile(PCR_I2C_CONF);
        core::ptr::write_volatile(PCR_I2C_CONF, i2c_conf | 1);
        let i2c_sclk = core::ptr::read_volatile(PCR_I2C_SCLK_CONF);
        core::ptr::write_volatile(PCR_I2C_SCLK_CONF, i2c_sclk | (1 << 22));

        // C61 I2CEXT0_SDA/SCL are matrix signals 47/46. GPIO6 is J1 pin 5
        // (SDA) and GPIO22 is J3 pin 6 (SCL) on DevKitC-1 v2.0.
        select_gpio_function(IO_MUX_GPIO6);
        select_gpio_function(IO_MUX_GPIO22);
        core::ptr::write_volatile(GPIO6_OUT_SEL, 47 | (1 << 10));
        core::ptr::write_volatile(GPIO22_OUT_SEL, 46 | (1 << 10));
        core::ptr::write_volatile(GPIO_I2C_SCL_IN_SEL, (1 << 8) | 22);
        core::ptr::write_volatile(GPIO_I2C_SDA_IN_SEL, (1 << 8) | 6);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO6_MASK | GPIO22_MASK);

        core::ptr::write_volatile(UART0_CLKDIV, 694);
        setup_i2c0();

        let mut chip_id = [0u8; 1];
        if !read_bme280_registers(0xD0, &mut chip_id) || chip_id[0] != 0x60 {
            uart_str(b"C61_BME280_FAIL CHIP_ID\n");
            wait_forever();
        }

        let mut temperature = [0u8; 3];
        if !read_bme280_registers(0xFA, &mut temperature) {
            uart_str(b"C61_BME280_FAIL TEMP_READ\n");
            wait_forever();
        }
        let raw_temperature = (u32::from(temperature[0]) << 12)
            | (u32::from(temperature[1]) << 4)
            | (u32::from(temperature[2]) >> 4);
        if raw_temperature != 0x082DC0 {
            uart_str(b"C61_BME280_FAIL TEMP\n");
            wait_forever();
        }

        uart_str(b"C61_BME280_OK ID=0x60 TEMP_RAW=0x082DC0 I2C0=GPIO6/22\n");
    }

    loop {
        core::hint::spin_loop();
    }
}

unsafe fn select_gpio_function(mux: *mut u32) {
    let pad = core::ptr::read_volatile(mux);
    core::ptr::write_volatile(mux, (pad & !(0x7 << 12)) | (1 << 12));
}

unsafe fn setup_i2c0() {
    // 40 MHz XTAL, 200+200 I2C module ticks = 100 kHz. The C61 model paces
    // this controller at its 160 MHz HP-core rate.
    core::ptr::write_volatile(I2C_CLK_CONF, 0);
    core::ptr::write_volatile(I2C_SCL_LOW_PERIOD, 199);
    core::ptr::write_volatile(I2C_SCL_HIGH_PERIOD, 199);
    core::ptr::write_volatile(I2C_SDA_HOLD, 2);
    core::ptr::write_volatile(I2C_SDA_SAMPLE, 2);
    core::ptr::write_volatile(I2C_SCL_START_HOLD, 10);
    core::ptr::write_volatile(I2C_SCL_RSTART_SETUP, 10);
    core::ptr::write_volatile(I2C_SCL_STOP_HOLD, 10);
    core::ptr::write_volatile(I2C_SCL_STOP_SETUP, 10);
}

unsafe fn read_bme280_registers(register: u8, destination: &mut [u8]) -> bool {
    if destination.is_empty() || destination.len() > 3 {
        return false;
    }

    core::ptr::write_volatile(I2C_FIFO_CONF, FIFO_RX_RST | FIFO_TX_RST);
    core::ptr::write_volatile(I2C_INT_CLR, u32::MAX);

    let command = |op: u32, count: u32| (op << 11) | count;
    core::ptr::write_volatile(I2C_CMD0, command(CMD_RSTART, 0));
    core::ptr::write_volatile(I2C_CMD0.add(1), command(CMD_WRITE, 2));
    core::ptr::write_volatile(I2C_CMD0.add(2), command(CMD_RSTART, 0));
    core::ptr::write_volatile(I2C_CMD0.add(3), command(CMD_WRITE, 1));
    core::ptr::write_volatile(
        I2C_CMD0.add(4),
        command(CMD_READ, destination.len() as u32) | CMD_ACK_VALUE,
    );
    core::ptr::write_volatile(I2C_CMD0.add(5), command(CMD_STOP, 0));

    core::ptr::write_volatile(I2C_DATA, u32::from(I2C_ADDRESS << 1));
    core::ptr::write_volatile(I2C_DATA, u32::from(register));
    core::ptr::write_volatile(I2C_DATA, u32::from((I2C_ADDRESS << 1) | 1));
    core::ptr::write_volatile(I2C_CTR, CTR_TRANS_START);

    let mut complete = false;
    for _ in 0..SPIN_LIMIT {
        if core::ptr::read_volatile(I2C_INT_RAW) & INT_TRANS_COMPLETE != 0 {
            complete = true;
            break;
        }
    }
    if !complete {
        return false;
    }

    let status = core::ptr::read_volatile(I2C_INT_RAW);
    core::ptr::write_volatile(I2C_INT_CLR, status);
    if status & INT_NACK != 0 {
        return false;
    }
    for byte in destination {
        *byte = core::ptr::read_volatile(I2C_DATA) as u8;
    }
    true
}

fn uart_str(bytes: &[u8]) {
    for &byte in bytes {
        unsafe { core::ptr::write_volatile(UART0_BASE, u32::from(byte)) };
    }
}

fn wait_forever() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
