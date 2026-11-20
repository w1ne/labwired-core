//! ESP32-C2 (ESP8684H4X) Embassy async GPIO smoke on esp-rtos. GPIO2 is
//! split into an `Input` and an `Output` driver on the same pad. A spawned
//! task drives one low-high-low pulse 1 ms after each request; main awaits
//! `wait_for_rising_edge` (woken by the esp-hal default GPIO handler through
//! matrix source 13) and records how long each wait took. With no pulse
//! requested, a `with_timeout` wait must expire (negative control). main
//! prints a PASS/FAIL verdict on UART0. Built image:
//! tests/fixtures/esp32c2-esp-hal-gpio-async.elf (see its .md for the build).
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{with_timeout, Duration, Instant, Timer};
use esp_backtrace as _;
use esp_hal::{
    gpio::{Flex, Output},
    timer::systimer::SystemTimer,
};
use esp_println::println;

const EDGES: u32 = 3;
const DELAY_MS: u64 = 1;

static GO: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[embassy_executor::task]
async fn driver(mut out: Output<'static>) {
    loop {
        GO.wait().await;
        Timer::after(Duration::from_millis(DELAY_MS)).await;
        out.set_high();
        Timer::after(Duration::from_millis(DELAY_MS)).await;
        out.set_low();
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());
    println!("LW-C2 gpio-async boot");

    let systimer = SystemTimer::new(p.SYSTIMER);
    esp_rtos::start(systimer.alarm0, p.FROM_CPU_INTR0);

    let mut pin = Flex::new(p.GPIO2);
    pin.set_low();
    pin.set_output_enable(true);
    // The pad is configured once here; only the Output half changes it.
    let (mut input, output) = unsafe { pin.split_into_drivers() };
    spawner.spawn(driver(output).unwrap());

    let mut edges = 0;
    let mut min_wait_us = u64::MAX;
    for _ in 0..EDGES {
        let start = Instant::now();
        GO.signal(());
        if with_timeout(Duration::from_millis(10), input.wait_for_rising_edge())
            .await
            .is_ok()
        {
            edges += 1;
            let wait_us = start.elapsed().as_micros();
            println!("LW-C2 edge wait_us {}", wait_us);
            min_wait_us = min_wait_us.min(wait_us);
        }
        // Let the pulse end before the next request.
        Timer::after(Duration::from_millis(2 * DELAY_MS)).await;
    }
    println!("LW-C2 rising edges {} min_wait_us {}", edges, min_wait_us);

    let timed_out = with_timeout(Duration::from_millis(2), input.wait_for_rising_edge())
        .await
        .is_err();
    println!("LW-C2 idle wait timed out {}", timed_out);

    let high_after = input.is_high();
    if edges == EDGES && min_wait_us >= DELAY_MS * 1000 && timed_out && !high_after {
        println!("LW-C2 gpio-async PASS");
    } else {
        println!("LW-C2 gpio-async FAIL");
    }
    loop {
        Timer::after(Duration::from_millis(100)).await;
    }
}
