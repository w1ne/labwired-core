#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const RESETS_BASE: usize = 0x4000_C000;
const RESETS_CLEAR: usize = RESETS_BASE + 0x3000;
const RESETS_DONE: usize = RESETS_BASE + 0x08;
const CLOCKS_BASE: usize = 0x4000_8000;
const CLK_PERI_CTRL: usize = CLOCKS_BASE + 0x48;
const IO_BANK0_BASE: usize = 0x4001_4000;
const I2C0_BASE: usize = 0x4004_4000;
const UART0_BASE: usize = 0x4003_4000;

const RESET_I2C0: u32 = 1 << 3;
const RESET_IO_BANK0: u32 = 1 << 5;
const RESET_PADS_BANK0: u32 = 1 << 8;
const RESET_UART0: u32 = 1 << 22;
const RESET_REQUIRED: u32 = RESET_I2C0 | RESET_IO_BANK0 | RESET_PADS_BANK0 | RESET_UART0;

const GPIO0: usize = 0;
const GPIO4: usize = 4;
const GPIO5: usize = 5;
const FUNC_UART0: u32 = 2;
const FUNC_I2C0: u32 = 3;

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

const UART0_DR: usize = UART0_BASE;
const UART0_IBRD: usize = UART0_BASE + 0x24;
const UART0_FBRD: usize = UART0_BASE + 0x28;
const UART0_LCR_H: usize = UART0_BASE + 0x2c;
const UART0_CR: usize = UART0_BASE + 0x30;
const CLK_PERI_ENABLE: u32 = 1 << 11;
const UART0_IBRD_115200: u32 = 3;
const UART0_FBRD_115200: u32 = 34;
const UART0_LCR_H_8N1_FIFO: u32 = (0b11 << 5) | (1 << 4);
const UART0_CR_ENABLE_TX: u32 = (1 << 0) | (1 << 8);

#[entry]
fn main() -> ! {
    labwired_entry()
}

#[no_mangle]
pub extern "C" fn labwired_entry() -> ! {
    initialize_required_blocks();
    uart0_init();

    match read_bme280_chip_id() {
        Ok(BME280_CHIP_ID) => write_uart(b"PICO_W_BME280_ID=0x60_I2C0_GP4_GP5\n"),
        Ok(_) => write_uart(b"PICO_W_BME280_FAIL=WRONG_ID\n"),
        Err(()) => write_uart(b"PICO_W_BME280_FAIL=I2C\n"),
    }

    loop {
        cortex_m::asm::nop();
    }
}

fn initialize_required_blocks() {
    unsafe {
        write(RESETS_CLEAR, RESET_REQUIRED);
        let mut ready = false;
        for _ in 0..100_000 {
            if read(RESETS_DONE) & RESET_REQUIRED == RESET_REQUIRED {
                ready = true;
                break;
            }
        }
        if !ready {
            write_uart(b"PICO_W_BME280_FAIL=RESET\n");
            loop {
                cortex_m::asm::nop();
            }
        }
    }
}

fn uart0_init() {
    unsafe {
        let peri = read(CLK_PERI_CTRL);
        write(CLK_PERI_CTRL, peri | CLK_PERI_ENABLE);
        write(gpio_ctrl(GPIO0), FUNC_UART0);
        write(UART0_IBRD, UART0_IBRD_115200);
        write(UART0_FBRD, UART0_FBRD_115200);
        write(UART0_LCR_H, UART0_LCR_H_8N1_FIFO);
        write(UART0_CR, UART0_CR_ENABLE_TX);
    }
}

fn read_bme280_chip_id() -> Result<u8, ()> {
    unsafe {
        // Pico W header pins 6/7 are GP4/GP5; FUNCSEL 3 selects I2C0.
        write(gpio_ctrl(GPIO4), FUNC_I2C0);
        write(gpio_ctrl(GPIO5), FUNC_I2C0);

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

        // Select CHIP_ID, then issue a repeated-start one-byte read and STOP.
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
            write(UART0_DR, u32::from(*byte));
        }
    }
}
