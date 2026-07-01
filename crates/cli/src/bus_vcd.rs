// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Export the universal bus-transaction trace (`SystemBus::bus_trace_snapshot`,
//! built by the generic I²C/SPI logic-analyzer wrappers in
//! `labwired_core::bus::bus_trace`) to formats outside the simulator:
//!
//! - JSON: the raw event list, for scripting / diffing / re-ingesting.
//! - VCD: a standard Value Change Dump, one 8-bit vector signal per distinct
//!   bus, so the capture opens directly in GTKWave, PulseView/sigrok, or any
//!   other waveform viewer. Each event becomes one value-change at `#<seq>`
//!   carrying the transacted byte (I²C: the `byte` field, already covering
//!   the address frame via `I2cSym::AddrWrite`/`AddrRead`; SPI: `mosi`).

use labwired_core::bus::bus_trace::{BusPayload, BusTraceEvent};
use std::collections::BTreeMap;
use std::io::{self, Write};
use vcd::{IdCode, TimescaleUnit, Value, VarType};

/// Write the raw trace events as pretty JSON.
pub fn write_bus_trace_json<W: Write>(events: &[BusTraceEvent], w: W) -> io::Result<()> {
    serde_json::to_writer_pretty(w, events).map_err(io::Error::other)
}

/// The byte a `BusTraceEvent` carries on the wire, for the VCD vector signal.
fn event_byte(payload: &BusPayload) -> u8 {
    match payload {
        BusPayload::I2c { byte, .. } => *byte,
        BusPayload::Spi { mosi, .. } => *mosi,
    }
}

fn byte_to_bits(byte: u8) -> [Value; 8] {
    let mut bits = [Value::V0; 8];
    for (i, bit) in bits.iter_mut().enumerate() {
        let shift = 7 - i; // MSB first
        *bit = if (byte >> shift) & 1 == 1 {
            Value::V1
        } else {
            Value::V0
        };
    }
    bits
}

/// Write a Value Change Dump: a `$timescale`, one `wire 8` `$var` per distinct
/// bus name (declaration order = first-seen order in `events`), then a
/// `#<seq>` / `b<binary> <id>` pair per event.
pub fn write_bus_trace_vcd<W: Write>(events: &[BusTraceEvent], sink: W) -> io::Result<()> {
    // Distinct bus names, first-seen order (stable, deterministic output).
    let mut bus_order: Vec<String> = Vec::new();
    for e in events {
        if !bus_order.contains(&e.bus) {
            bus_order.push(e.bus.clone());
        }
    }

    let mut writer = vcd::Writer::new(sink);
    writer.timescale(1, TimescaleUnit::US)?;
    writer.add_module("bus_trace")?;
    let mut ids: BTreeMap<String, IdCode> = BTreeMap::new();
    for bus in &bus_order {
        let id = writer.add_var(VarType::Wire, 8, bus, None)?;
        ids.insert(bus.clone(), id);
    }
    writer.upscope()?;
    writer.enddefinitions()?;

    for e in events {
        let id = ids[&e.bus];
        writer.timestamp(e.seq)?;
        writer.change_vector(id, byte_to_bits(event_byte(&e.payload)))?;
    }
    writer.flush()
}
