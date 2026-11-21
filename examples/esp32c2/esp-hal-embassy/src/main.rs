//! ESP32-C2 (ESP8684H4X) Embassy async smoke on esp-rtos. The esp-rtos
//! scheduler takes SYSTIMER alarm 0 as its time driver and FROM_CPU_INTR0
//! for task switches. A spawned task awaits three 1 ms `Timer`s and signals
//! main; main checks the tick count and the elapsed `Instant`, then checks
//! that `with_timeout` on a signal nobody sends expires (negative control).
//! main prints a PASS/FAIL verdict on UART0. Built image:
//! tests/fixtures/esp32c2-esp-hal-embassy.elf (see its .md for the build).
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{with_timeout, Duration, Instant, Timer};
use esp_backtrace as _;
use esp_hal::timer::systimer::SystemTimer;
use esp_println::println;

const TICKS: u32 = 3;
const PERIOD_MS: u64 = 1;

static DONE: Signal<CriticalSectionRawMutex, u32> = Signal::new();
static NEVER: Signal<CriticalSectionRawMutex, ()> = Signal::new();

#[embassy_executor::task]
async fn ticker() {
    let mut ticks = 0;
    for _ in 0..TICKS {
        Timer::after(Duration::from_millis(PERIOD_MS)).await;
        ticks += 1;
    }
    DONE.signal(ticks);
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());
    println!("LW-C2 embassy boot");

    let systimer = SystemTimer::new(p.SYSTIMER);
    esp_rtos::start(systimer.alarm0, p.FROM_CPU_INTR0);

    let start = Instant::now();
    spawner.spawn(ticker().unwrap());
    let ticks = DONE.wait().await;
    let elapsed_us = start.elapsed().as_micros();
    println!("LW-C2 ticks {} elapsed_us {}", ticks, elapsed_us);

    let timed_out = with_timeout(Duration::from_millis(1), NEVER.wait())
        .await
        .is_err();
    println!("LW-C2 timeout fired {}", timed_out);

    let min_us = TICKS as u64 * PERIOD_MS * 1000;
    if ticks == TICKS && elapsed_us >= min_us && timed_out {
        println!("LW-C2 embassy PASS");
    } else {
        println!("LW-C2 embassy FAIL");
    }
    loop {
        Timer::after(Duration::from_millis(100)).await;
    }
}
