// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! End-to-end egress-bridge proof.
//!
//! Drives the REAL UART `push_tx` fan-out that firmware register writes trigger
//! — a simulated firmware writing a sensor reading to its UART TX register —
//! and asserts the exact bytes arrive over a real localhost TCP socket, the
//! same way a user's dashboard/backend would receive them.
//!
//! Net-gated (opens a real socket): run with
//! `cargo test -p labwired-core --features net-tests --test egress_e2e`.
#![cfg(feature = "net-tests")]

use labwired_core::network::egress::bus::EgressBus;
use labwired_core::network::egress::tap::EgressTap;
use labwired_core::network::egress::transport::TcpSink;
use labwired_core::network::egress::{BufferPolicy, EgressItem, EncodingKind};
use labwired_core::network::Interconnect;
use labwired_core::peripherals::uart::Uart;
use labwired_core::Peripheral;
use std::io::Read;
use std::net::TcpListener;
use std::sync::mpsc::channel;

/// Stm32F1 UART: writing the legacy TX alias at offset 0x00 pushes a byte onto
/// the TX line (through `push_tx`, which fans out to attached stream devices).
const TX_REG: u64 = 0x00;

#[test]
fn firmware_uart_output_reaches_a_real_tcp_backend() {
    let reading = b"TEMP=21.5C\n";

    // A stand-in "customer backend" listening on localhost.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let want = reading.to_vec();
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut got = vec![0u8; want.len()];
        sock.read_exact(&mut got).unwrap();
        got
    });

    // Egress bridge: raw bytes → real TCP transport.
    let (tx, rx) = channel::<EgressItem>();
    let sink = TcpSink::connect(&addr).unwrap();
    let mut bus = EgressBus::new(
        rx,
        EncodingKind::Raw,
        BufferPolicy::default(),
        Box::new(sink),
    );

    // A simulated UART with the egress tap attached (as `from_manifest` wires it).
    let mut uart = Uart::new();
    uart.attach_stream(Box::new(EgressTap::new(tx)));

    // "Firmware" writes the reading byte-by-byte to its TX register.
    for &b in reading {
        uart.write(TX_REG, b).unwrap();
    }

    // One sim step drains the tap and hands the payload to the transport worker.
    bus.tick().unwrap();

    assert_eq!(
        server.join().unwrap(),
        reading.to_vec(),
        "the exact firmware UART output must arrive on the real socket"
    );
}
