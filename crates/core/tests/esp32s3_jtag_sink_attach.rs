// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Regression guard: the S3 USB Serial/JTAG block must land its EP1 bytes in
//! the capture sink attached by `attach_uart_tx_sink`. An Arduino build with
//! ARDUINO_USB_CDC_ON_BOOT writes `Serial` here, and a run whose sink is not on
//! the object the bus routes writes to reports "no serial" for firmware that
//! printed — the exact shape that stalled hosted ESP32-S3 proving.

use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::Bus;
use std::sync::{Arc, Mutex};

#[test]
fn s3_jtag_ep1_bytes_reach_the_attached_sink() {
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    // EP1 @ 0x6003_8000: a 32-bit store of 'A' (the IDF driver's write shape).
    bus.write_u32(0x6003_8000, u32::from(b'A')).expect("write EP1");
    // A second byte through the byte path.
    bus.write_u8(0x6003_8000, b'B').expect("write EP1 byte");

    let captured = sink.lock().expect("sink lock").clone();
    assert_eq!(captured, b"AB", "JTAG EP1 bytes must reach the capture sink");
}

#[test]
fn s3_uart0_tx_bytes_reach_the_attached_sink() {
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    // UART0 TX FIFO @ 0x6000_0000 — the address an Arduino UART0 `Serial`
    // writes. The bank also installs a `low_mmio` catch-all over 0x6000_0000;
    // if that wins the decode, app console bytes vanish with no sink.
    bus.write_u32(0x6000_0000, u32::from(b'X')).expect("write UART0 fifo");
    for _ in 0..40000 {
        bus.tick_peripherals_with_costs();
    }

    let captured = sink.lock().expect("sink lock").clone();
    assert!(
        captured.contains(&b'X'),
        "UART0 TX must reach the capture sink; got {:?}",
        captured
    );
}
