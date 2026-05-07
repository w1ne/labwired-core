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
// Threshold in 0.01 °C units to avoid pulling in the FPU (the simulator
// doesn't model FP yet). 30.00 °C → 3000.
const THRESHOLD_CENTI_C: i32 = 3000;

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

    // GPIO8 = SDA, GPIO9 = SCL. ESP32-S3 has no fixed-function I²C pins —
    // any GPIO routes through the GPIO matrix. On real hardware, install
    // external 4.7 kΩ pull-ups to 3V3: TMP102 breakouts typically don't
    // include them, and esp-hal's default config does not enable internal
    // pull-ups. The simulator's I²C model is forgiving of missing pulls.
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
            match i2c.write_read(TMP102_ADDR, &[0x00], &mut buf) {
                Ok(()) => {
                    // Combine the 2-byte big-endian register read into a 16-bit
                    // value. Cast through u32 to make the bit pattern unambiguous
                    // (compiler-fused i16-cast paths produced wrong magic-number
                    // divisions when the firmware was first ported). Top 12 bits
                    // are the temperature value, in 1/16 °C units.
                    let raw_u: u32 = ((buf[0] as u32) << 8) | (buf[1] as u32);
                    let units_16: i32 = (raw_u >> 4) as i32;
                    // Each unit = 0.0625 °C. Multiply by 625 then divide by 100
                    // to get centi-degrees as integer (no FPU on this build).
                    let centi_c: i32 = units_16 * 625 / 100;
                    let int_part = centi_c / 100;
                    let frac_part = centi_c.unsigned_abs() % 100;
                    println!("T = {}.{:02} C", int_part, frac_part);
                    if centi_c > THRESHOLD_CENTI_C {
                        led.set_high();
                    } else {
                        led.set_low();
                    }
                }
                Err(e) => {
                    println!("I2C error: {:?}", e);
                }
            }
        }
        core::hint::spin_loop();
    }
}
