#![no_std]
#![no_main]

use cortex_m_rt::entry;

#[entry]
fn main() -> ! {
    // Initialize I2C
    let i2c = init_i2c();

    // Read DEVID register (should be 0xE5)
    let devid = i2c_read(0x53, 0x00);
    assert_eq!(devid, 0xE5, "DEVID mismatch");

    // Configure BW_RATE to 100Hz
    i2c_write(0x53, 0x2C, 0x0A);

    // Enable measurement mode
    i2c_write(0x53, 0x2D, 0x08);

    // Read acceleration data
    loop {
        let x_lsb = i2c_read(0x53, 0x32);
        let x_msb = i2c_read(0x53, 0x33);
        let x_accel = ((x_msb as i16) << 8) | (x_lsb as i16);

        // Process acceleration data
        delay_ms(100);
    }
}

fn i2c_read(addr: u8, reg: u8) -> u8 {
    // I2C read implementation
    0
}

fn i2c_write(addr: u8, reg: u8, val: u8) {
    // I2C write implementation
}

fn init_i2c() -> () {
    // I2C initialization
}

fn delay_ms(ms: u32) {
    // Delay implementation
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
