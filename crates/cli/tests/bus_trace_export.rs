// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Task 4: bus-trace export (`--bus-trace-out`) — VCD + JSON writers.
//!
//! These exercise the writer functions directly (`labwired_cli::bus_vcd`);
//! the CLI wiring (`--bus-trace-out <path>` on `labwired run`) is covered by
//! manual smoke per the task brief, since it needs a full chip + firmware run.

#[test]
fn vcd_export_emits_one_signal_per_bus_with_byte_values() {
    use labwired_core::bus::bus_trace::{BusPayload, BusTraceEvent, I2cSym};
    let events = vec![
        BusTraceEvent {
            seq: 1,
            cycle: 100,
            bus: "i2c1".into(),
            payload: BusPayload::I2c {
                kind: I2cSym::AddrWrite,
                byte: 0x3C,
                ack: true,
            },
        },
        BusTraceEvent {
            seq: 2,
            cycle: 200,
            bus: "i2c1".into(),
            payload: BusPayload::I2c {
                kind: I2cSym::Data,
                byte: 0xAF,
                ack: true,
            },
        },
    ];
    let mut out = Vec::new();
    labwired_cli::bus_vcd::write_bus_trace_vcd(&events, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("$var"), "declares VCD vars");
    assert!(text.contains("i2c1"), "names the bus");
    assert!(
        text.contains("b10101111") || text.contains("AF") || text.to_lowercase().contains("af"),
        "encodes 0xAF"
    );
}

#[test]
fn json_export_round_trips_events() {
    use labwired_core::bus::bus_trace::{BusPayload, BusTraceEvent};
    let events = vec![BusTraceEvent {
        seq: 1,
        cycle: 100,
        bus: "spi0".into(),
        payload: BusPayload::Spi {
            mosi: 0x10,
            miso: 0x20,
        },
    }];
    let mut out = Vec::new();
    labwired_cli::bus_vcd::write_bus_trace_json(&events, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed[0]["bus"], "spi0");
    assert_eq!(parsed[0]["payload"]["mosi"], 16);
}

/// UART octets belong in the VCD; CAN frames deliberately do not.
///
/// A UART event is one octet on a wire, which is exactly what a `wire 8`
/// signal represents. A CAN event is a whole frame — arbitration id, DLC, up
/// to 64 payload bytes — and no single octet of it was ever on the wire as a
/// byte-wide sample. Emitting one anyway (the first data byte, or a zero) would
/// put a value in a waveform viewer that never existed, and a waveform is read
/// as measurement. So CAN is absent from the VCD entirely — not even a declared
/// signal, because a `wire 8` that is never written renders in GTKWave as a
/// channel sitting at its initial value, i.e. "this bus was idle" rather than
/// "this bus is not representable here".
#[test]
fn vcd_carries_uart_octets_and_omits_frame_oriented_can() {
    use labwired_core::bus::bus_trace::{BusDir, BusPayload, BusTraceEvent};
    let events = vec![
        BusTraceEvent {
            seq: 1,
            cycle: 10,
            bus: "uart0".into(),
            payload: BusPayload::Uart {
                direction: BusDir::Tx,
                byte: 0xAF,
            },
        },
        BusTraceEvent {
            seq: 2,
            cycle: 20,
            bus: "fdcan1".into(),
            payload: BusPayload::Can {
                direction: BusDir::Tx,
                id: 0x7E0,
                data: vec![0x03, 0x22],
                extended: false,
                fd: false,
                bitrate_switch: false,
                remote: false,
            },
        },
    ];

    let mut out = Vec::new();
    labwired_cli::bus_vcd::write_bus_trace_vcd(&events, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();

    assert!(text.contains("uart0"), "the UART bus gets a signal");
    assert!(text.contains("b10101111"), "and its octet 0xAF is encoded");
    assert!(
        !text.contains("fdcan1"),
        "a frame-oriented bus must not be declared as a wire-8 signal it can \
         never write to — that reads as an idle channel, not an absent one:\n{text}"
    );

    // The JSON export is lossless, so nothing is actually lost by the omission.
    let mut json = Vec::new();
    labwired_cli::bus_vcd::write_bus_trace_json(&events, &mut json).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8(json).unwrap()).unwrap();
    assert_eq!(parsed[0]["payload"]["protocol"], "uart");
    assert_eq!(parsed[0]["payload"]["direction"], "tx");
    assert_eq!(parsed[1]["payload"]["protocol"], "can");
    assert_eq!(parsed[1]["payload"]["id"], 0x7E0);
    assert_eq!(parsed[1]["payload"]["data"][1], 0x22);
}
