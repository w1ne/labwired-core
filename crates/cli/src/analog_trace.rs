// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Export the in-core analog engine's waveform trace
//! (`Machine::analog_trace_snapshot`) to a file:
//!
//! - `.csv` — `time_ns,<channel>...`, one row per co-simulation step. Plots
//!   anywhere, diffs in CI, and is what the validation matrix compares.
//! - anything else (`.vcd`) — a Value Change Dump with one `real` variable per
//!   channel, so the analog curve opens in GTKWave / PulseView beside the
//!   digital logic capture on the same time axis.
//!
//! Both formats use nanoseconds, the engine's own unit, so no rounding is
//! introduced between the solver and the file.

use labwired_core::analog::AnalogTraceBatch;
use std::io::{self, Write};
use std::path::Path;
use vcd::{TimescaleUnit, VarType};

/// Write `batch` to `path`, picking CSV or VCD from the extension.
pub fn write_analog_trace(batch: &AnalogTraceBatch, path: &Path) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    let is_csv = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"));
    if is_csv {
        write_analog_csv(batch, file)
    } else {
        write_analog_vcd(batch, file)
    }
}

/// `time_ns,<channel>...` with one row per sample.
pub fn write_analog_csv<W: Write>(batch: &AnalogTraceBatch, mut sink: W) -> io::Result<()> {
    write!(sink, "time_ns")?;
    for channel in &batch.channels {
        write!(sink, ",{}", csv_field(&channel.name))?;
    }
    writeln!(sink)?;
    for sample in &batch.samples {
        write!(sink, "{}", sample.time_ns)?;
        for value in &sample.values {
            write!(sink, ",{value}")?;
        }
        writeln!(sink)?;
    }
    Ok(())
}

/// One `real` variable per channel, timestamped in nanoseconds.
pub fn write_analog_vcd<W: Write>(batch: &AnalogTraceBatch, sink: W) -> io::Result<()> {
    let mut writer = vcd::Writer::new(sink);
    writer.timescale(1, TimescaleUnit::NS)?;
    writer.add_module("analog")?;
    let mut ids = Vec::with_capacity(batch.channels.len());
    for channel in &batch.channels {
        // The unit rides in the name because VCD has nowhere else to put it,
        // and a scope channel without its unit is a number without a meaning.
        let name = format!("{}_{}", vcd_identifier(&channel.name), channel.unit);
        ids.push(writer.add_var(VarType::Real, 64, &name, None)?);
    }
    writer.upscope()?;
    writer.enddefinitions()?;

    let mut previous: Vec<Option<f32>> = vec![None; batch.channels.len()];
    for sample in &batch.samples {
        writer.timestamp(sample.time_ns)?;
        for (index, value) in sample.values.iter().enumerate() {
            // A VCD records changes, not samples: emitting an unchanged value
            // every step would quadruple the file and tell a reader nothing.
            if previous.get(index).copied().flatten() == Some(*value) {
                continue;
            }
            if let Some(id) = ids.get(index) {
                writer.change_real(*id, f64::from(*value))?;
            }
            if let Some(slot) = previous.get_mut(index) {
                *slot = Some(*value);
            }
        }
    }
    Ok(())
}

/// Quote a CSV field only when it needs it.
fn csv_field(name: &str) -> String {
    if name.contains([',', '"', '\n']) {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        name.to_string()
    }
}

/// VCD identifiers cannot carry spaces or the `(` `)` of a probe expression.
fn vcd_identifier(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
