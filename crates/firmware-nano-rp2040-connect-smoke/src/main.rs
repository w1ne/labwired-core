#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const IO_BANK0_GPIO0_CTRL: *mut u32 = 0x4001_4004 as *mut u32;
const IO_BANK0_GPIO12_CTRL: *mut u32 = 0x4001_4064 as *mut u32;
const IO_BANK0_GPIO13_CTRL: *mut u32 = 0x4001_406C as *mut u32;
const SIO_GPIO_OUT: *mut u32 = 0xD000_0010 as *mut u32;
const SIO_GPIO_OE: *mut u32 = 0xD000_0020 as *mut u32;
const UART0: usize = 0x4003_4000;
const UART0_DR: *mut u32 = UART0 as *mut u32;
const UART0_IBRD: *mut u32 = (UART0 + 0x24) as *mut u32;
const UART0_FBRD: *mut u32 = (UART0 + 0x28) as *mut u32;
const UART0_LCR_H: *mut u32 = (UART0 + 0x2C) as *mut u32;
const UART0_CR: *mut u32 = (UART0 + 0x30) as *mut u32;
const I2C0: usize = 0x4004_4000;
const I2C_CON: *mut u32 = (I2C0 + 0x00) as *mut u32;
const I2C_TAR: *mut u32 = (I2C0 + 0x04) as *mut u32;
const I2C_DATA_CMD: *mut u32 = (I2C0 + 0x10) as *mut u32;
const I2C_SS_SCL_HCNT: *mut u32 = (I2C0 + 0x14) as *mut u32;
const I2C_SS_SCL_LCNT: *mut u32 = (I2C0 + 0x18) as *mut u32;
const I2C_RAW_INTR_STAT: *const u32 = (I2C0 + 0x34) as *const u32;
const I2C_CLR_TX_ABRT: *const u32 = (I2C0 + 0x54) as *const u32;
const I2C_ENABLE: *mut u32 = (I2C0 + 0x6C) as *mut u32;
const I2C_RXFLR: *const u32 = (I2C0 + 0x78) as *const u32;
const I2C_TX_ABRT_SOURCE: *const u32 = (I2C0 + 0x80) as *const u32;

const GPIO0_BIT: u32 = 1;
const GPIO_FUNC_SIO: u32 = 5;
const GPIO_FUNC_UART: u32 = 2;
const GPIO_FUNC_I2C: u32 = 3;
const LSM6DSOX_ADDRESS: u32 = 0x6A;
const DATA_CMD_READ: u32 = 1 << 8;
const DATA_CMD_STOP: u32 = 1 << 9;
const DATA_CMD_RESTART: u32 = 1 << 10;
const INTR_TX_ABRT: u32 = 1 << 6;
const MAX_POLLS: usize = 1_000_000;

const WHO_AM_I: u8 = 0x0F;
const CTRL1_XL: u8 = 0x10;
const CTRL1_XL_416_HZ_2_G: u8 = 0x60;
const STATUS_REG: u8 = 0x1E;
const XLDA: u8 = 1;
const OUTX_L_A: u8 = 0x28;

#[entry]
fn main() -> ! {
    unsafe {
        setup_pins();
        setup_i2c();
        blink_header_tx();

        let who = read_register(WHO_AM_I);
        let configured_write = write_register(CTRL1_XL, CTRL1_XL_416_HZ_2_G);
        let configured_readback = read_register(CTRL1_XL);
        let ready = wait_for_accelerometer_data();
        let axes = read_accelerometer_xyz();

        setup_console_pin();
        if who == Some(0x6C)
            && configured_write
            && configured_readback == Some(CTRL1_XL_416_HZ_2_G)
            && ready
            && axes == Some((1000, -2000, 4000))
        {
            uart_write(b"NANO_RP2040_LSM6DSOX_ID=0x6C XYZ=1000,-2000,4000 PASS\n");
        } else if who != Some(0x6C) {
            uart_write(b"NANO_RP2040_LSM6DSOX_FAIL_WHOAMI\n");
        } else if !configured_write {
            uart_write(b"NANO_RP2040_LSM6DSOX_FAIL_CONFIG_WRITE\n");
        } else if configured_readback != Some(CTRL1_XL_416_HZ_2_G) {
            uart_write(b"NANO_RP2040_LSM6DSOX_FAIL_CONFIG_READBACK\n");
        } else if !ready {
            uart_write(b"NANO_RP2040_LSM6DSOX_FAIL_DATA_READY\n");
        } else {
            uart_write(b"NANO_RP2040_LSM6DSOX_FAIL_XYZ\n");
        }
    }

    loop {
        cortex_m::asm::nop();
    }
}

unsafe fn setup_pins() {
    core::ptr::write_volatile(IO_BANK0_GPIO0_CTRL, GPIO_FUNC_SIO);
    core::ptr::write_volatile(IO_BANK0_GPIO12_CTRL, GPIO_FUNC_I2C);
    core::ptr::write_volatile(IO_BANK0_GPIO13_CTRL, GPIO_FUNC_I2C);
    core::ptr::write_volatile(SIO_GPIO_OE, GPIO0_BIT);
}

unsafe fn setup_i2c() {
    core::ptr::write_volatile(I2C_ENABLE, 0);
    core::ptr::write_volatile(
        I2C_CON,
        1 /* master */ | (1 << 1) /* standard speed */ | (1 << 5) /* restart */ | (1 << 6),
    );
    core::ptr::write_volatile(I2C_SS_SCL_HCNT, 625);
    core::ptr::write_volatile(I2C_SS_SCL_LCNT, 875);
    core::ptr::write_volatile(I2C_TAR, LSM6DSOX_ADDRESS);
    core::ptr::write_volatile(I2C_ENABLE, 1);
}

unsafe fn blink_header_tx() {
    for _ in 0..5 {
        core::ptr::write_volatile(SIO_GPIO_OUT, GPIO0_BIT);
        delay();
        core::ptr::write_volatile(SIO_GPIO_OUT, 0);
        delay();
    }
}

unsafe fn setup_console_pin() {
    core::ptr::write_volatile(IO_BANK0_GPIO0_CTRL, GPIO_FUNC_UART);
    core::ptr::write_volatile(UART0_CR, 0);
    core::ptr::write_volatile(UART0_IBRD, 72);
    core::ptr::write_volatile(UART0_FBRD, 8);
    core::ptr::write_volatile(UART0_LCR_H, (0b11 << 5) | (1 << 4));
    core::ptr::write_volatile(UART0_CR, 1 | (1 << 8) | (1 << 9));
}

unsafe fn read_register(register: u8) -> Option<u8> {
    core::ptr::write_volatile(I2C_DATA_CMD, u32::from(register));
    wait_tx_empty()?;
    check_address_ack()?;

    core::ptr::write_volatile(
        I2C_DATA_CMD,
        DATA_CMD_READ | DATA_CMD_RESTART | DATA_CMD_STOP,
    );
    wait_rx_byte()?;
    check_address_ack()?;
    Some((core::ptr::read_volatile(I2C_DATA_CMD) & 0xFF) as u8)
}

unsafe fn write_register(register: u8, value: u8) -> bool {
    core::ptr::write_volatile(I2C_DATA_CMD, u32::from(register));
    core::ptr::write_volatile(I2C_DATA_CMD, u32::from(value) | DATA_CMD_STOP);
    wait_tx_empty().is_some() && check_address_ack().is_some()
}

unsafe fn read_accelerometer_xyz() -> Option<(i16, i16, i16)> {
    let mut bytes = [0; 6];
    read_registers(OUTX_L_A, &mut bytes)?;
    Some((
        i16::from_le_bytes([bytes[0], bytes[1]]),
        i16::from_le_bytes([bytes[2], bytes[3]]),
        i16::from_le_bytes([bytes[4], bytes[5]]),
    ))
}

unsafe fn read_registers(register: u8, out: &mut [u8]) -> Option<()> {
    if out.is_empty() {
        return Some(());
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
        *byte = (core::ptr::read_volatile(I2C_DATA_CMD) & 0xFF) as u8;
    }
    Some(())
}

unsafe fn wait_for_accelerometer_data() -> bool {
    for _ in 0..MAX_POLLS {
        match read_register(STATUS_REG) {
            Some(status) if status & XLDA != 0 => return true,
            Some(_) => {}
            None => return false,
        }
    }
    false
}

unsafe fn wait_tx_empty() -> Option<()> {
    for _ in 0..MAX_POLLS {
        if core::ptr::read_volatile(I2C_RAW_INTR_STAT) & (1 << 4) != 0 {
            return Some(());
        }
    }
    None
}

unsafe fn wait_rx_byte() -> Option<()> {
    for _ in 0..MAX_POLLS {
        if core::ptr::read_volatile(I2C_RXFLR) != 0 {
            return Some(());
        }
    }
    None
}

unsafe fn check_address_ack() -> Option<()> {
    if core::ptr::read_volatile(I2C_RAW_INTR_STAT) & INTR_TX_ABRT != 0 {
        let _ = core::ptr::read_volatile(I2C_CLR_TX_ABRT);
        return None;
    }
    if core::ptr::read_volatile(I2C_TX_ABRT_SOURCE) & 1 != 0 {
        return None;
    }
    Some(())
}

fn uart_write(bytes: &[u8]) {
    for byte in bytes {
        unsafe { core::ptr::write_volatile(UART0_DR, u32::from(*byte)) };
    }
}

#[inline(never)]
fn delay() {
    for _ in 0..5_000 {
        cortex_m::asm::nop();
    }
}
