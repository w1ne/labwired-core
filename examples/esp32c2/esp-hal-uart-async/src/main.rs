//! ESP32-C2 (ESP8684H4X) Embassy async UART smoke on esp-rtos. UART1 runs
//! in loopback mode (UART_CONF0.LOOPBACK, bit 14), so every byte the TX
//! shift register sends arrives in the RX FIFO of the same UART; no pad or
//! test-script stimulus is needed. A spawned task writes a 16-byte frame
//! with `write_async` 1 ms after each request; main awaits
//! `read_exact_async` (woken by the esp-hal UART1 async handler through
//! matrix source 18) and checks the bytes and the wait time. With no frame
//! requested, a `with_timeout` read must expire (negative control). main
//! waits 5 ms before each request so the UART0 console has drained. main
//! prints a PASS/FAIL verdict on UART0. Built image:
//! tests/fixtures/esp32c2-esp-hal-uart-async.elf (see its .md for the build).
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{with_timeout, Duration, Instant, Timer};
use esp_backtrace as _;
use esp_hal::{
    timer::systimer::SystemTimer,
    uart::{Config, Uart, UartTx},
    Async,
};
use esp_println::println;

const FRAMES: u32 = 3;
const DELAY_MS: u64 = 1;
const FRAME_LEN: usize = 16;

/// UART1 CONF0 (esp32c2 uart_reg.h: DR_REG_UART1_BASE 0x60010000 + 0x20).
const UART1_CONF0: *mut u32 = 0x6001_0020 as *mut u32;
const CONF0_LOOPBACK: u32 = 1 << 14;

static GO: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// Printable bytes, so the loopback traffic stays readable where a capture
/// merges UART1 into the UART0 log.
fn frame(seq: u8) -> [u8; FRAME_LEN] {
    let mut f = [0u8; FRAME_LEN];
    for (i, b) in f.iter_mut().enumerate() {
        *b = b'a' + (seq * 5 + i as u8 * 3) % 26;
    }
    f
}

#[embassy_executor::task]
async fn writer(mut tx: UartTx<'static, Async>) {
    loop {
        let seq = GO.wait().await;
        Timer::after(Duration::from_millis(DELAY_MS)).await;
        let f = frame(seq);
        let mut sent = 0;
        while sent < FRAME_LEN {
            sent += tx.write_async(&f[sent..]).await.unwrap();
        }
        tx.flush_async().await.unwrap();
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());
    println!("LW-C2 uart-async boot");

    let systimer = SystemTimer::new(p.SYSTIMER);
    esp_rtos::start(systimer.alarm0, p.FROM_CPU_INTR0);

    let uart = Uart::new(p.UART1, Config::default()).unwrap().into_async();
    // Internal TX->RX loopback; esp-hal has no API for it.
    unsafe { UART1_CONF0.write_volatile(UART1_CONF0.read_volatile() | CONF0_LOOPBACK) };
    let (mut rx, tx) = uart.split();
    spawner.spawn(writer(tx).unwrap());

    let mut good = 0;
    let mut min_wait_us = u64::MAX;
    for seq in 0..FRAMES as u8 {
        let mut buf = [0u8; FRAME_LEN];
        Timer::after(Duration::from_millis(5)).await;
        let start = Instant::now();
        GO.signal(seq);
        if with_timeout(Duration::from_millis(10), rx.read_exact_async(&mut buf))
            .await
            .is_ok_and(|r| r.is_ok())
        {
            let wait_us = start.elapsed().as_micros();
            let ok = buf == frame(seq);
            println!("LW-C2 frame {} wait_us {} match {}", seq, wait_us, ok);
            if ok {
                good += 1;
                min_wait_us = min_wait_us.min(wait_us);
            }
        }
    }
    Timer::after(Duration::from_millis(5)).await;
    println!("LW-C2 frames {} min_wait_us {}", good, min_wait_us);

    let mut one = [0u8; 1];
    let timed_out = with_timeout(Duration::from_millis(2), rx.read_async(&mut one))
        .await
        .is_err();
    println!("LW-C2 idle read timed out {}", timed_out);

    // At 115200 baud a 16-byte frame takes 16 * 10 / 115200 s = 1389 us.
    if good == FRAMES && min_wait_us >= DELAY_MS * 1000 + 1389 && timed_out {
        println!("LW-C2 uart-async PASS");
    } else {
        println!("LW-C2 uart-async FAIL");
    }
    loop {
        Timer::after(Duration::from_millis(100)).await;
    }
}
