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
