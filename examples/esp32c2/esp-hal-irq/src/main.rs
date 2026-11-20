//! ESP32-C2 (ESP8684H4X) esp-hal interrupt smoke: a FROM_CPU software
//! interrupt and a periodic SYSTIMER alarm, each with an `#[handler]` bound
//! through the interrupt matrix. Both handlers count; main prints the counts
//! and a PASS/FAIL verdict on UART0. Built image:
//! tests/fixtures/esp32c2-esp-hal-irq.elf (see its .md for the build).
#![no_std]
#![no_main]

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use critical_section::Mutex;
use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    handler,
    interrupt::software::SoftwareInterrupt,
    main,
    time::Duration,
    timer::{systimer::SystemTimer, PeriodicTimer},
    Blocking,
};
use esp_println::println;

static SWI0: Mutex<RefCell<Option<SoftwareInterrupt<'static, 0>>>> =
    Mutex::new(RefCell::new(None));
static ALARM: Mutex<RefCell<Option<PeriodicTimer<'static, Blocking>>>> =
    Mutex::new(RefCell::new(None));
// RV32IMC has no A extension, so no fetch_add; each counter has one writer.
static SWI_HITS: AtomicU32 = AtomicU32::new(0);
static ALARM_HITS: AtomicU32 = AtomicU32::new(0);

#[handler]
fn swi0_handler() {
    critical_section::with(|cs| {
        if let Some(swi) = SWI0.borrow_ref(cs).as_ref() {
            swi.reset();
        }
    });
    SWI_HITS.store(SWI_HITS.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
}

#[handler]
fn alarm_handler() {
    critical_section::with(|cs| {
        if let Some(timer) = ALARM.borrow_ref_mut(cs).as_mut() {
            timer.clear_interrupt();
        }
    });
    ALARM_HITS.store(ALARM_HITS.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    println!("LW-C2 irq boot");

    let mut swi = SoftwareInterrupt::new(p.FROM_CPU_INTR0);
    critical_section::with(|cs| {
        swi.set_interrupt_handler(swi0_handler);
        SWI0.borrow_ref_mut(cs).replace(swi);
    });
    for _ in 0..3 {
        critical_section::with(|cs| {
            if let Some(swi) = SWI0.borrow_ref(cs).as_ref() {
                swi.raise();
            }
        });
    }
    let swi_hits = SWI_HITS.load(Ordering::Relaxed);
    println!("LW-C2 swi hits {}", swi_hits);

    let systimer = SystemTimer::new(p.SYSTIMER);
    let mut timer = PeriodicTimer::new(systimer.alarm0);
    critical_section::with(|cs| {
        timer.set_interrupt_handler(alarm_handler);
        timer.listen();
        timer.start(Duration::from_millis(1)).unwrap();
        ALARM.borrow_ref_mut(cs).replace(timer);
    });
    Delay::new().delay_millis(5);
    let alarm_hits = ALARM_HITS.load(Ordering::Relaxed);
    println!("LW-C2 alarm hits {}", alarm_hits);

    if swi_hits == 3 && (4..=6).contains(&alarm_hits) {
        println!("LW-C2 irq PASS");
    } else {
        println!("LW-C2 irq FAIL");
    }
    loop {}
}
