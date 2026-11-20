//! ESP32-C2 (ESP8684H4X) esp-hal GPIO interrupt smoke. GPIO2 is a `Flex`
//! pin with output and input both enabled, so the pad drives its own input
//! buffer. An `Io` `#[handler]` (matrix source 13) counts the edges that
//! latch `GPIO_STATUS` in three phases: rising edge only, any edge, and
//! not listening (the negative control). main prints the counts and a
//! PASS/FAIL verdict on UART0. Built image:
//! tests/fixtures/esp32c2-esp-hal-gpio-irq.elf (see its .md for the build).
#![no_std]
#![no_main]

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use critical_section::Mutex;
use esp_backtrace as _;
use esp_hal::{
    gpio::{Event, Flex, Io},
    handler, main,
};
use esp_println::println;

static PIN: Mutex<RefCell<Option<Flex<'static>>>> = Mutex::new(RefCell::new(None));
// RV32IMC has no A extension, so no fetch_add; the counter has one writer.
static HITS: AtomicU32 = AtomicU32::new(0);

#[handler]
fn gpio_handler() {
    critical_section::with(|cs| {
        if let Some(pin) = PIN.borrow_ref_mut(cs).as_mut() {
            if pin.is_interrupt_set() {
                pin.clear_interrupt();
                HITS.store(HITS.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
            }
        }
    });
}

/// Drive `pulses` low-high-low pulses on the loopback pin and return the
/// number of handler runs they caused.
fn pulse(pulses: u32) -> u32 {
    let before = HITS.load(Ordering::Relaxed);
    for _ in 0..pulses {
        for high in [true, false] {
            critical_section::with(|cs| {
                if let Some(pin) = PIN.borrow_ref_mut(cs).as_mut() {
                    if high {
                        pin.set_high();
                    } else {
                        pin.set_low();
                    }
                }
            });
        }
    }
    HITS.load(Ordering::Relaxed) - before
}

fn listen(event: Option<Event>) {
    critical_section::with(|cs| {
        if let Some(pin) = PIN.borrow_ref_mut(cs).as_mut() {
            match event {
                Some(event) => pin.listen(event),
                None => pin.unlisten(),
            }
        }
    });
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    println!("LW-C2 gpio-irq boot");

    let mut io = Io::new(p.IO_MUX);
    io.set_interrupt_handler(gpio_handler);

    let mut pin = Flex::new(p.GPIO2);
    pin.set_low();
    pin.set_output_enable(true);
    pin.set_input_enable(true);
    critical_section::with(|cs| {
        PIN.borrow_ref_mut(cs).replace(pin);
    });

    listen(Some(Event::RisingEdge));
    let rising = pulse(3);
    println!("LW-C2 rising hits {}", rising);

    listen(Some(Event::AnyEdge));
    let any = pulse(2);
    println!("LW-C2 any-edge hits {}", any);

    listen(None);
    let off = pulse(2);
    println!("LW-C2 unlistened hits {}", off);

    if rising == 3 && any == 4 && off == 0 {
        println!("LW-C2 gpio-irq PASS");
    } else {
        println!("LW-C2 gpio-irq FAIL");
    }
    loop {}
}
