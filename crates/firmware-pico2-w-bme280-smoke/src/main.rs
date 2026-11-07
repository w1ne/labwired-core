#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2350 address map. The controller uses the shared DW_apb_i2c model.
const UART0_DR: *mut u32 = 0x4007_0000 as *mut u32;
const I2C0: usize = 0x4009_0000;
const I2C_CON: *mut u32 = (I2C0 + 0x00) as *mut u32;
const I2C_TAR: *mut u32 = (I2C0 + 0x04) as *mut u32;
const I2C_DATA_CMD: *mut u32 = (I2C0 + 0x10) as *mut u32;
const I2C_SS_SCL_HCNT: *mut u32 = (I2C0 + 0x14) as *mut u32;
const I2C_SS_SCL_LCNT: *mut u32 = (I2C0 + 0x18) as *mut u32;
const I2C_RAW_INTR_STAT: *const u32 = (I2C0 + 0x34) as *const u32;
const I2C_CLR_TX_ABRT: *const u32 = (I2C0 + 0x54) as *const u32;
const I2C_ENABLE: *mut u32 = (I2C0 + 0x6c) as *mut u32;
const I2C_RXFLR: *const u32 = (I2C0 + 0x78) as *const u32;
const I2C_TX_ABRT_SOURCE: *const u32 = (I2C0 + 0x80) as *const u32;

const IO_BANK0_GPIO4_CTRL: *mut u32 = 0x4002_8024 as *mut u32;
const IO_BANK0_GPIO5_CTRL: *mut u32 = 0x4002_802c as *mut u32;
const GPIO_FUNC_I2C: u32 = 3;

const BME280_ADDRESS: u32 = 0x77;
const DATA_CMD_READ: u32 = 1 << 8;
const DATA_CMD_STOP: u32 = 1 << 9;
const DATA_CMD_RESTART: u32 = 1 << 10;
const INTR_TX_ABRT: u32 = 1 << 6;
const MAX_POLLS: usize = 20_000;

#[entry]
fn main() -> ! {
    unsafe {
        setup_pins();
        setup_i2c();

        match read_register(0xD0) {
            Ok(0x60) => print_uart("BME280_ID=0x60\n"),
            Ok(id) => {
                print_uart("BME280_ID_ERROR=0x");
                print_hex_byte(id);
                print_uart("\n");
                halt();
            }
            Err(()) => {
                print_uart("BME280_I2C_ERROR\n");
                halt();
            }
        }

        let mut temp = [0u8; 3];
        if read_registers(0xFA, &mut temp).is_err() {
            print_uart("BME280_I2C_ERROR\n");
            halt();
        }
        let raw =
            (u32::from(temp[0]) << 12) | (u32::from(temp[1]) << 4) | (u32::from(temp[2]) >> 4);
        print_uart("BME280_TEMP_RAW=0x");
        print_hex20(raw);
        print_uart("\nPICO2W_BME280_SMOKE_OK\n");
    }

    loop {
        cortex_m::asm::wfi();
    }
}

unsafe fn setup_pins() {
    // Pico 2 W header route: GP4 = I2C0 SDA, GP5 = I2C0 SCL.
    core::ptr::write_volatile(IO_BANK0_GPIO4_CTRL, GPIO_FUNC_I2C);
    core::ptr::write_volatile(IO_BANK0_GPIO5_CTRL, GPIO_FUNC_I2C);
}

unsafe fn setup_i2c() {
    // Disable while configuring the DesignWare master for standard-mode I2C.
    core::ptr::write_volatile(I2C_ENABLE, 0);
    core::ptr::write_volatile(
        I2C_CON,
        1 /* master */ | (1 << 1) /* standard speed */ | (1 << 5) /* restart */ | (1 << 6),
    );
    core::ptr::write_volatile(I2C_SS_SCL_HCNT, 625);
    core::ptr::write_volatile(I2C_SS_SCL_LCNT, 875);
    core::ptr::write_volatile(I2C_TAR, BME280_ADDRESS);
    core::ptr::write_volatile(I2C_ENABLE, 1);
}

unsafe fn read_register(register: u8) -> Result<u8, ()> {
    core::ptr::write_volatile(I2C_DATA_CMD, u32::from(register));
    wait_tx_empty()?;
    check_address_ack()?;

    core::ptr::write_volatile(
        I2C_DATA_CMD,
        DATA_CMD_READ | DATA_CMD_RESTART | DATA_CMD_STOP,
    );
    wait_rx_byte()?;
    check_address_ack()?;
    Ok((core::ptr::read_volatile(I2C_DATA_CMD) & 0xff) as u8)
}

unsafe fn read_registers(register: u8, out: &mut [u8]) -> Result<(), ()> {
    if out.is_empty() {
        return Ok(());
    }
    let len = out.len();
    core::ptr::write_volatile(I2C_DATA_CMD, u32::from(register));
    wait_tx_empty()?;
    check_address_ack()?;

    for (index, byte) in out.iter_mut().enumerate() {
        let mut command = DATA_CMD_READ;
        if index == 0 {
            command |= DATA_CMD_RESTART;
        }
        if index + 1 == len {
            command |= DATA_CMD_STOP;
        }
        core::ptr::write_volatile(I2C_DATA_CMD, command);
        wait_rx_byte()?;
        check_address_ack()?;
        *byte = (core::ptr::read_volatile(I2C_DATA_CMD) & 0xff) as u8;
    }
    Ok(())
}

unsafe fn wait_tx_empty() -> Result<(), ()> {
    for _ in 0..MAX_POLLS {
        if core::ptr::read_volatile(I2C_RAW_INTR_STAT) & (1 << 4) != 0 {
            return Ok(());
        }
    }
    Err(())
}

unsafe fn wait_rx_byte() -> Result<(), ()> {
    for _ in 0..MAX_POLLS {
        if core::ptr::read_volatile(I2C_RXFLR) != 0 {
            return Ok(());
        }
    }
    Err(())
}

unsafe fn check_address_ack() -> Result<(), ()> {
    if core::ptr::read_volatile(I2C_RAW_INTR_STAT) & INTR_TX_ABRT != 0 {
        let _ = core::ptr::read_volatile(I2C_CLR_TX_ABRT);
        return Err(());
    }
    if core::ptr::read_volatile(I2C_TX_ABRT_SOURCE) & 1 != 0 {
        return Err(());
    }
    Ok(())
}

fn print_uart(text: &str) {
    for byte in text.bytes() {
        unsafe { core::ptr::write_volatile(UART0_DR, u32::from(byte)) };
    }
}

fn print_hex_byte(value: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    unsafe {
        core::ptr::write_volatile(UART0_DR, u32::from(HEX[(value >> 4) as usize]));
        core::ptr::write_volatile(UART0_DR, u32::from(HEX[(value & 0x0f) as usize]));
    }
}

fn print_hex20(value: u32) {
    print_hex_byte(((value >> 16) & 0xff) as u8);
    print_hex_byte(((value >> 8) & 0xff) as u8);
    print_hex_byte((value & 0xff) as u8);
}

fn halt() -> ! {
    loop {
        cortex_m::asm::wfi();
    }
}
