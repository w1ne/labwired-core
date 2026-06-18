// Task 2: prove SystemBus::attach_uart_stream_by_id wires a *live* stream onto a
// named UART (the seam World::from_manifest uses to wire UartCrossLink endpoints).

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::uart::UartStreamDevice;
use std::sync::{Arc, Mutex};

struct Recorder(Arc<Mutex<Vec<u8>>>);
impl UartStreamDevice for Recorder {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        None
    }
    fn on_tx_byte(&mut self, byte: u8) {
        self.0.lock().unwrap().push(byte);
    }
}

fn l476_bus() -> SystemBus {
    let chip_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    );
    let chip = labwired_config::ChipDescriptor::from_file(chip_path).unwrap();
    let manifest: labwired_config::SystemManifest =
        serde_yaml::from_str("name: seam-test\nchip: ignored\n").unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

#[test]
fn attach_uart_stream_by_id_wires_a_live_stream() {
    let mut bus = l476_bus();
    let seen = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_stream_by_id("uart2", Box::new(Recorder(seen.clone())))
        .expect("uart2 should accept a stream device");

    // uart2 base 0x40004400, V2 layout TDR at offset 0x28 → writing it transmits.
    bus.write_u32(0x4000_4428, 0x42).unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![0x42]);
}

#[test]
fn attach_uart_stream_by_id_rejects_unknown_and_non_uart() {
    let mut bus = l476_bus();
    assert!(bus
        .attach_uart_stream_by_id(
            "does_not_exist",
            Box::new(Recorder(Arc::new(Mutex::new(Vec::new()))))
        )
        .is_err());
    // spi1 exists on the L476 but is not a UART → must be rejected.
    assert!(bus
        .attach_uart_stream_by_id("spi1", Box::new(Recorder(Arc::new(Mutex::new(Vec::new())))))
        .is_err());
}
