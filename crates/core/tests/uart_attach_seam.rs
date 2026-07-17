// Task 2: prove SystemBus::attach_uart_stream_by_id wires a *live* stream onto a
// named UART (the seam World::from_manifest uses to wire UartCrossLink endpoints).

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::uart::UartStreamDevice;
use labwired_core::Bus;
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

/// Enable the USART1/USART2 peripheral clocks (RCC_APB2ENR.USART1EN bit 14,
/// RCC_APB1ENR1.USART2EN bit 17). On the L476 these are clock-gated and unclocked
/// out of reset (RM0351), so a bare TDR write is ignored until firmware ungates
/// them — these register-poking seam tests must do what firmware does.
fn enable_l476_uart_clocks(bus: &mut SystemBus) {
    const RCC: u64 = 0x4002_1000;
    bus.write_u32(RCC + 0x60, 1 << 14).unwrap(); // APB2ENR: USART1EN
    bus.write_u32(RCC + 0x58, 1 << 17).unwrap(); // APB1ENR1: USART2EN
}

#[test]
fn attach_uart_stream_by_id_wires_a_live_stream() {
    let mut bus = l476_bus();
    enable_l476_uart_clocks(&mut bus);
    let seen = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_stream_by_id("uart2", Box::new(Recorder(seen.clone())))
        .expect("uart2 should accept a stream device");

    // uart2 base 0x40004400, V2 layout TDR at offset 0x28 → writing it transmits.
    bus.write_u32(0x4000_4428, 0x42).unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![0x42]);
}

#[test]
fn detach_uart_sink_by_id_keeps_crosslink_bytes_out_of_the_console() {
    let mut bus = l476_bus();
    enable_l476_uart_clocks(&mut bus);
    let console = Arc::new(Mutex::new(Vec::new()));
    // The console sink is attached to EVERY UART (as the wasm bridge does).
    bus.attach_uart_tx_sink(console.clone(), false);

    // uart1 (debug) keeps feeding the console; uart2 (cross-link) is excluded.
    bus.detach_uart_sink_by_id("uart2")
        .expect("uart2 should be detachable from the sink");

    // uart1 base 0x40013800, V2 TDR at offset 0x28.
    bus.write_u32(0x4001_3828, 0x41).unwrap();
    // uart2 base 0x40004400, V2 TDR at offset 0x28 — protocol byte, must NOT
    // reach the console.
    bus.write_u32(0x4000_4428, 0x99).unwrap();

    assert_eq!(
        *console.lock().unwrap(),
        vec![0x41],
        "only the debug UART (uart1) should reach the console; the cross-link \
         (uart2) protocol byte must be excluded"
    );
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
