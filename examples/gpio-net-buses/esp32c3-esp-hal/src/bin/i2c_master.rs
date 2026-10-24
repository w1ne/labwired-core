//! esp-hal 1.1 I2C0 master on gpio nets (ESP32-C3): the exchange of
//! `src/esp32c_i2c_controller.c` through esp-hal's blocking driver, against
//! the G071 target at 0x42 (`env-i2c-esp32c3.yaml` with this firmware).
//! GPIO8 SDA, GPIO9 SCL (esp-hal: open drain, pull-up), 100 kHz.
//!
//! Console (USB-Serial-JTAG): `C3 write true`, `C3 read true DE AD`,
//! `C3 nack Err(AcknowledgeCheckFailed(Address))`, `C3 I2C DONE`.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    i2c::master::{Config, I2c},
    main,
    time::Rate,
};
use esp_println::println;

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    let mut i2c = I2c::new(p.I2C0, Config::default().with_frequency(Rate::from_khz(100)))
        .unwrap()
        .with_sda(p.GPIO8)
        .with_scl(p.GPIO9);
    println!("C3 I2C BOOT");
    let r = i2c.write(0x42u8, &[0x10, 0xDE, 0xAD]);
    println!("C3 write {:?}", r.is_ok());
    let mut buf = [0u8; 2];
    let r = i2c.write_read(0x42u8, &[0x10], &mut buf);
    println!("C3 read {:?} {:02X} {:02X}", r.is_ok(), buf[0], buf[1]);
    let r = i2c.write(0x50u8, &[0x55]);
    println!("C3 nack {:?}", r);
    println!("C3 I2C DONE");
    loop {}
}
