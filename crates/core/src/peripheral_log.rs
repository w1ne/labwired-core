// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Named text logs that peripheral models record during a run.
//!
//! Some models keep a record of what happened: a simulated USB host logs the
//! enumeration it did, a flash controller logs the commands it executed, a
//! shifter logs the words it put on a pin. A model gives these records through
//! [`crate::Peripheral::logs`], one text line per entry. An entry can stand
//! for a run of identical events: it has a repeat count, and its rendered line
//! ends in ` x{count}` when the count is more than 1. A long poll loop then
//! costs one entry, not millions. The bus adds one more
//! log to every peripheral: [`BUS_TRACE`], the lines of the universal bus
//! trace that carry the peripheral's name. See
//! [`crate::bus::SystemBus::peripheral_logs`].
//!
//! `labwired test` asserts on these lines with `peripheral_log`. The names a
//! model returns are the only list of valid names: a script that names a log
//! the model does not return is a config error.

/// The name of the bus-trace log that every peripheral has. Its lines are the
/// one-line payload summaries of [`crate::bus::bus_trace::BusPayload`], for
/// example `addr 0x54 W nack` or `mosi 0x9f miso 0xef`.
pub const BUS_TRACE: &str = "bus_trace";

/// One entry of a log: the text of one event and how many times in a row it
/// happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// The line for one event, e.g. `seq 2 cmd 0x05 addr 0x00000000 size 1`.
    pub text: String,
    /// Consecutive identical events this entry stands for. At least 1.
    pub count: u64,
}

impl LogEntry {
    pub fn new(text: impl Into<String>, count: u64) -> Self {
        Self {
            text: text.into(),
            count: count.max(1),
        }
    }
}

/// The rendered line: `text`, then ` x{count}` when the count is more than 1.
/// A substring of the single-event line (`text`) is also a substring of the
/// rendered line.
impl std::fmt::Display for LogEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)?;
        if self.count > 1 {
            write!(f, " x{}", self.count)?;
        }
        Ok(())
    }
}

/// One named log of a peripheral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeripheralLog {
    /// The name a test script uses, e.g. `host`.
    pub name: &'static str,
    /// The recorded entries, oldest first.
    pub entries: Vec<LogEntry>,
}

impl PeripheralLog {
    /// A log with one entry per line (every count 1).
    pub fn new(name: &'static str, lines: Vec<String>) -> Self {
        let entries = lines.into_iter().map(|l| LogEntry::new(l, 1)).collect();
        Self { name, entries }
    }

    /// A log of run-length entries.
    pub fn from_entries(name: &'static str, entries: Vec<LogEntry>) -> Self {
        Self { name, entries }
    }

    /// The rendered lines, one per entry (see [`LogEntry`]'s `Display`).
    pub fn lines(&self) -> Vec<String> {
        self.entries.iter().map(ToString::to_string).collect()
    }

    /// Events that contain `needle`: the sum of the counts of the entries
    /// whose `text` contains it. A run of N identical events counts N.
    pub fn count_matching(&self, needle: &str) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.text.contains(needle))
            .map(|e| e.count)
            .sum()
    }

    /// All events in the log: the sum of the entry counts.
    pub fn events(&self) -> u64 {
        self.entries.iter().map(|e| e.count).sum()
    }
}

impl crate::bus::SystemBus {
    /// Every named log of the peripheral `name`: the logs the model records
    /// ([`crate::Peripheral::logs`]), then [`BUS_TRACE`]. `None` when no
    /// peripheral and no attached device has that name.
    ///
    /// The bus trace is a ring. When it is full the oldest events go first,
    /// so a long run can lose early lines.
    /// [`crate::bus::bus_trace::BusTrace::evicted`] tells how many.
    ///
    /// When no peripheral has that name, `name` can be the
    /// `external_devices:` id of an external device, which has only the logs
    /// it records and no [`BUS_TRACE`] of its own: a bus-resident device (a
    /// GPIO part such as a segment display), or a device attached to a
    /// controller ([`crate::peripherals::i2c::I2cDevice::logs`]; its traffic is
    /// in the bus trace of that controller).
    pub fn peripheral_logs(&self, name: &str) -> Option<Vec<PeripheralLog>> {
        let Some(index) = self.find_peripheral_index_by_name(name) else {
            return self
                .gpio_devices
                .iter()
                .find(|d| d.id() == name)
                .map(|d| d.logs())
                .or_else(|| self.device_logs(name));
        };
        let mut logs = self.peripherals[index].dev.logs();
        let trace = self
            .bus_trace
            .snapshot()
            .into_iter()
            .filter(|e| e.bus == name)
            .map(|e| e.payload.to_string())
            .collect();
        logs.push(PeripheralLog::new(BUS_TRACE, trace));
        Some(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::bus_trace::{BusPayload, I2cSym};
    use crate::bus::SystemBus;
    use crate::peripherals::imxrt::flexspi::ImxrtFlexspi;

    #[test]
    fn model_logs_then_the_bus_trace_of_that_peripheral() {
        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "flash",
            0x4000_0000,
            0x1000,
            None,
            Box::new(ImxrtFlexspi::default()),
        );
        let nack = BusPayload::I2c {
            kind: I2cSym::AddrWrite,
            byte: 0x54 << 1,
            ack: false,
        };
        bus.bus_trace.push("flash", nack.clone());
        bus.bus_trace.push("i2c9", nack);

        let logs = bus.peripheral_logs("flash").expect("known peripheral");
        let names: Vec<_> = logs.iter().map(|l| l.name).collect();
        assert_eq!(names, ["ip", BUS_TRACE], "empty logs are still listed");
        assert!(logs[0].entries.is_empty());
        assert_eq!(
            logs[1].lines(),
            ["addr 0x54 W nack"],
            "other buses filtered out"
        );
        assert_eq!(bus.peripheral_logs("nope"), None);
    }

    #[test]
    fn entry_renders_count_only_above_one() {
        assert_eq!(
            LogEntry::new("cmd 0x05 size 1", 1).to_string(),
            "cmd 0x05 size 1"
        );
        assert_eq!(
            LogEntry::new("cmd 0x05 size 1", 7).to_string(),
            "cmd 0x05 size 1 x7"
        );
        assert_eq!(LogEntry::new("a", 0).count, 1, "count is at least 1");
    }

    #[test]
    fn count_matching_sums_repeat_counts() {
        let log = PeripheralLog::from_entries(
            "ip",
            vec![
                LogEntry::new("cmd 0x06 addr 0x00000000 size 0", 1),
                LogEntry::new("cmd 0x05 addr 0x00000000 size 1", 5000),
                LogEntry::new("cmd 0x20 addr 0x00082000 size 0", 1),
            ],
        );
        assert_eq!(log.count_matching("cmd 0x05 "), 5000);
        assert_eq!(log.count_matching("cmd 0x20 addr 0x00082000 "), 1);
        assert_eq!(log.count_matching("cmd 0x9f "), 0, "negative control");
        assert_eq!(
            log.count_matching("x5000"),
            0,
            "the count suffix is not text"
        );
        assert_eq!(log.events(), 5002);
        assert_eq!(log.entries.len(), 3);
    }

    #[test]
    fn plain_lines_count_one_each() {
        let log = PeripheralLog::new("wire", vec!["pin 2 ".into(), "pin 2 ".into()]);
        assert_eq!(log.count_matching("pin 2"), 2);
        assert_eq!(log.lines(), ["pin 2 ", "pin 2 "]);
    }
}
