//! ESP32-S3 blinky for the LabWired simulator (Plan 3).
//!
//! Toggles GPIO2 from a SYSTIMER alarm ISR every 500 ms.  Runs identically
//! on the simulator and on a connected ESP32-S3-Zero (probe with a logic
//! analyzer or multimeter on GPIO2 to see the toggle).
//!
//! In the simulator, the GpioObserver registered by `labwired run` emits
//! a tracing event for each transition.

#![no_std]
#![no_main]

use core::cell::RefCell;
use critical_section::Mutex;
use esp_backtrace as _;
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    handler, main,
    time::Duration,
    timer::{systimer::SystemTimer, PeriodicTimer},
};

static LED: Mutex<RefCell<Option<Output<'static>>>> =
    Mutex::new(RefCell::new(None));
static ALARM: Mutex<RefCell<Option<PeriodicTimer<'static, esp_hal::Blocking>>>> =
    Mutex::new(RefCell::new(None));

#[handler]
fn alarm_isr() {
    critical_section::with(|cs| {
        if let Some(led) = LED.borrow_ref_mut(cs).as_mut() {
            led.toggle();
        }
        if let Some(alarm) = ALARM.borrow_ref_mut(cs).as_mut() {
            alarm.clear_interrupt();
        }
    });
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());

    // Output on GPIO2.
    let led = Output::new(p.GPIO2, Level::Low, OutputConfig::default());

    // Periodic SYSTIMER alarm @ 500 ms.
    let st = SystemTimer::new(p.SYSTIMER);
    let mut alarm = PeriodicTimer::new(st.alarm0);
    alarm.set_interrupt_handler(alarm_isr);
    alarm.start(Duration::from_millis(500)).unwrap();
    alarm.listen();

    critical_section::with(|cs| {
        LED.replace(cs, Some(led));
        ALARM.replace(cs, Some(alarm));
    });

    loop {
        core::hint::spin_loop();
    }
}
