#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2350 address map from Raspberry Pi's RP2350 datasheet / pico-sdk headers.
const RESETS_BASE: usize = 0x4002_0000;
const RESETS_CLR: usize = RESETS_BASE + 0x3000;
const RESETS_DONE: usize = RESETS_BASE + 0x08;
const IO_BANK0_BASE: usize = 0x4002_8000;
const I2C0_BASE: usize = 0x4009_0000;
const UART0_DR: *mut u32 = 0x4007_0000 as *mut u32;

const RESET_I2C0: u32 = 1 << 3;
const RESET_IO_BANK0: u32 = 1 << 5;
const RESET_PADS_BANK0: u32 = 1 << 8;
const RESET_REQUIRED: u32 = RESET_I2C0 | RESET_IO_BANK0 | RESET_PADS_BANK0;

const GPIO4: usize = 4;
const GPIO5: usize = 5;
const FUNC_I2C: u32 = 3;

const I2C_CON: usize = 0x00;
const I2C_TAR: usize = 0x04;
const I2C_DATA_CMD: usize = 0x10;
const I2C_SCL_HCNT: usize = 0x14;
const I2C_SCL_LCNT: usize = 0x18;
const I2C_STATUS: usize = 0x34;
const I2C_CLR_INTR: usize = 0x40;
const I2C_CLR_TX_ABRT: usize = 0x54;
const I2C_CLR_STOP_DET: usize = 0x60;
const I2C_ENABLE: usize = 0x6c;

const I2C_READ: u32 = 1 << 8;
const I2C_STOP: u32 = 1 << 9;
const I2C_RESTART: u32 = 1 << 10;
const I2C_TX_ABRT: u32 = 1 << 6;
const I2C_STOP_DETECTED: u32 = 1 << 9;
const BME280_ADDRESS: u32 = 0x77;
const BME280_CHIP_ID_REGISTER: u32 = 0xd0;
const BME280_CHIP_ID: u8 = 0x60;

#[entry]
fn main() -> ! {
    let result = read_bme280_chip_id();
    match result {
        Ok(BME280_CHIP_ID) => write_uart(b"PICO2W_BME280_ID=0x60_I2C0_GP4_GP5\n"),
        Ok(other) => write_uart(if other == 0 {
            b"PICO2W_BME280_FAIL=ID_0x00\n"
        } else {
            b"PICO2W_BME280_FAIL=WRONG_ID\n"
        }),
        Err(()) => write_uart(b"PICO2W_BME280_FAIL=I2C\n"),
    }

    loop {
        cortex_m::asm::nop();
    }
}

fn read_bme280_chip_id() -> Result<u8, ()> {
    unsafe {
        write(RESETS_CLR, RESET_REQUIRED);
        let mut ready = false;
        for _ in 0..100_000 {
            if read(RESETS_DONE) & RESET_REQUIRED == RESET_REQUIRED {
                ready = true;
                break;
            }
        }
        if !ready {
            return Err(());
        }

        // RP2350 IO_BANK0 routes GP4/GP5 to I2C0 with FUNCSEL 3.
        write(gpio_ctrl(GPIO4), FUNC_I2C);
        write(gpio_ctrl(GPIO5), FUNC_I2C);

        // Disable, configure a standard-mode master with repeated starts, then
        // target the BME280's default 7-bit address.
        write(I2C0_BASE + I2C_ENABLE, 0);
        write(
            I2C0_BASE + I2C_CON,
            1 | (1 << 1) | (1 << 5) | (1 << 6) | (1 << 8),
        );
        write(I2C0_BASE + I2C_SCL_HCNT, 600);
        write(I2C0_BASE + I2C_SCL_LCNT, 650);
        write(I2C0_BASE + I2C_TAR, BME280_ADDRESS);
        write(I2C0_BASE + I2C_ENABLE, 1);
        let _ = read(I2C0_BASE + I2C_CLR_INTR);

        // Write CHIP_ID's register address, then issue one repeated-start read
        // that also closes the transaction with STOP.
        write(I2C0_BASE + I2C_DATA_CMD, BME280_CHIP_ID_REGISTER);
        write(I2C0_BASE + I2C_DATA_CMD, I2C_READ | I2C_RESTART | I2C_STOP);
        for _ in 0..100_000 {
            let status = read(I2C0_BASE + I2C_STATUS);
            if status & I2C_TX_ABRT != 0 {
                let _ = read(I2C0_BASE + I2C_CLR_TX_ABRT);
                return Err(());
            }
            if status & I2C_STOP_DETECTED != 0 {
                let _ = read(I2C0_BASE + I2C_CLR_STOP_DET);
                return Ok(read(I2C0_BASE + I2C_DATA_CMD) as u8);
            }
        }
    }
    Err(())
}

const fn gpio_ctrl(gpio: usize) -> usize {
    IO_BANK0_BASE + 0x04 + gpio * 8
}

unsafe fn read(address: usize) -> u32 {
    (address as *const u32).read_volatile()
}

unsafe fn write(address: usize, value: u32) {
    (address as *mut u32).write_volatile(value);
}

fn write_uart(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            UART0_DR.write_volatile(u32::from(*byte));
        }
    }
}
