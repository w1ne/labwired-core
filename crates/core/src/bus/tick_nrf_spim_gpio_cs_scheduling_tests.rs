use super::SystemBus;
use crate::peripherals::{
    gpio::{GpioPort, GpioRegisterLayout},
    spi::{Spi, SpiDevice, SpiRegisterLayout},
};
use crate::Bus;
use std::sync::{Arc, Mutex};

const GPIO0: u64 = 0x5000_0000;
const SPIM2: u64 = 0x4002_3000;

struct TransactionDevice(Arc<Mutex<Vec<&'static str>>>);

impl SpiDevice for TransactionDevice {
    fn transfer(&mut self, _mosi: u8) -> u8 {
        0
    }

    fn cs_pin(&self) -> &str {
        "P0.12"
    }

    fn cs_select(&mut self) {
        self.0.lock().unwrap().push("select");
    }

    fn cs_release(&mut self) {
        self.0.lock().unwrap().push("release");
    }
}

#[test]
fn system_bus_keeps_selected_spim_ticked_until_gpio_release() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut bus = SystemBus::new();
    bus.add_peripheral(
        "gpio0",
        GPIO0,
        0x1000,
        None,
        Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Nrf52)),
    );
    bus.add_peripheral(
        "spim2",
        SPIM2,
        0x1000,
        None,
        Box::new(Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim)),
    );
    bus.attach_spi_device("spim2", Box::new(TransactionDevice(events.clone())))
        .unwrap();

    Bus::write_u32(&mut bus, GPIO0 + 0x518, 1 << 12).unwrap(); // DIRSET
    Bus::write_u32(&mut bus, GPIO0 + 0x508, 1 << 12).unwrap(); // OUTSET: idle high
    Bus::write_u8(&mut bus, 0x2000_0200, 0xA5).unwrap();
    Bus::write_u32(&mut bus, SPIM2 + 0x500, 7).unwrap();
    Bus::write_u32(&mut bus, SPIM2 + 0x544, 0x2000_0200).unwrap();
    Bus::write_u32(&mut bus, SPIM2 + 0x548, 1).unwrap();

    Bus::write_u32(&mut bus, GPIO0 + 0x50C, 1 << 12).unwrap(); // OUTCLR
    Bus::write_u32(&mut bus, SPIM2 + 0x010, 1).unwrap(); // TASKS_START
    Bus::tick_peripherals(&mut bus);
    assert_eq!(*events.lock().unwrap(), ["select"]);

    Bus::write_u32(&mut bus, GPIO0 + 0x508, 1 << 12).unwrap(); // OUTSET
    Bus::tick_peripherals(&mut bus);
    assert_eq!(*events.lock().unwrap(), ["select", "release"]);
    assert!(
        bus.bus_tick_indices.is_empty(),
        "released SPIM must self-remove"
    );

    // A new low edge before the next START begins a distinct transaction.
    Bus::write_u32(&mut bus, GPIO0 + 0x50C, 1 << 12).unwrap();
    Bus::write_u32(&mut bus, SPIM2 + 0x010, 1).unwrap();
    Bus::tick_peripherals(&mut bus);
    Bus::write_u32(&mut bus, GPIO0 + 0x508, 1 << 12).unwrap();
    Bus::tick_peripherals(&mut bus);
    assert_eq!(
        *events.lock().unwrap(),
        ["select", "release", "select", "release"]
    );
    assert!(
        bus.bus_tick_indices.is_empty(),
        "second release must self-remove"
    );
}
