//! ESP32-S3 I2C + TMP102 demo for the LabWired simulator (Plan 4).
//!
//! Reads temperature from a TMP102 over I2C0 once per second, prints it
//! over USB-Serial-JTAG, and toggles GPIO2 high when temperature exceeds
//! 30 C. Runs identically on the simulator (against the simulated TMP102
//! at 0x48) and on real silicon (with a TMP102 wired to GPIO8/GPIO9).

#![no_std]
#![no_main]

use core::cell::RefCell;
use critical_section::Mutex;
use esp_backtrace as _;
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    handler,
    i2c::master::{Config as I2cConfig, I2c},
    main,
    time::Duration,
    timer::{systimer::SystemTimer, PeriodicTimer},
};
use esp_println::println;

const TMP102_ADDR: u8 = 0x48;
const THRESHOLD_C: f32 = 30.0;

static TICK_FLAG: Mutex<RefCell<bool>> = Mutex::new(RefCell::new(false));
static ALARM: Mutex<RefCell<Option<PeriodicTimer<'static, esp_hal::Blocking>>>> =
    Mutex::new(RefCell::new(None));

#[handler]
fn alarm_isr() {
    critical_section::with(|cs| {
        TICK_FLAG.replace(cs, true);
        if let Some(alarm) = ALARM.borrow_ref_mut(cs).as_mut() {
            alarm.clear_interrupt();
        }
    });
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());

    let mut led = Output::new(p.GPIO2, Level::Low, OutputConfig::default());

    let mut i2c = I2c::new(p.I2C0, I2cConfig::default())
        .unwrap()
        .with_sda(p.GPIO8)
        .with_scl(p.GPIO9);

    let st = SystemTimer::new(p.SYSTIMER);
    let mut alarm = PeriodicTimer::new(st.alarm0);
    alarm.set_interrupt_handler(alarm_isr);
    alarm.start(Duration::from_millis(1000)).unwrap();
    alarm.listen();

    critical_section::with(|cs| {
        ALARM.replace(cs, Some(alarm));
    });

    loop {
        let tick = critical_section::with(|cs| {
            let v = *TICK_FLAG.borrow_ref(cs);
            TICK_FLAG.replace(cs, false);
            v
        });
        if tick {
            let mut buf = [0u8; 2];
            if i2c.write_read(TMP102_ADDR, &[0x00], &mut buf).is_ok() {
                let raw = ((buf[0] as i16) << 8) | (buf[1] as i16);
                let temp_c = (raw >> 4) as f32 * 0.0625;
                println!("T = {:.2} °C", temp_c);
                if temp_c > THRESHOLD_C {
                    led.set_high();
                } else {
                    led.set_low();
                }
            }
        }
        core::hint::spin_loop();
    }
}
