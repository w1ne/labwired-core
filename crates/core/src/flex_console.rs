// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The console of a Flex manifest: the bytes portable firmware writes through
//! `labwired.console_write`, where target firmware prints over a UART or a
//! USB serial port.
//!
//! Only the bytes and the virtual time of each write are modeled. A write
//! takes no virtual time; there is no baud rate, framing, TX pin, USB
//! transfer, or input to the firmware.

use labwired_config::FlexConsole;
use serde::Serialize;

/// Bytes the console keeps. Later bytes are counted in `dropped_bytes`, and a
/// `console_contains` check cannot see them.
pub const FLEX_CONSOLE_MAX_BYTES: usize = 1 << 20;

const NOT_MODELED: &[&str] = &[
    "baud_rate_and_transfer_time",
    "uart_framing_and_tx_pin",
    "usb_enumeration_and_host_polling",
    "input_to_firmware",
];

/// What a run report states about the console, including what it leaves out.
#[derive(Debug, Clone, Serialize)]
pub struct FlexConsoleSummary {
    pub name: String,
    pub not_modeled: &'static [&'static str],
}

/// One `console_write` call: its kept bytes are
/// `text` bytes `offset..offset + len`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FlexConsoleWrite {
    pub time_ns: u64,
    pub offset: usize,
    pub len: usize,
}

/// The console output of a run, for `result.json` and the browser.
#[derive(Debug, Clone, Serialize)]
pub struct FlexConsoleCapture {
    pub name: String,
    pub time_unit: &'static str,
    /// The kept bytes as UTF-8; an invalid sequence reads as U+FFFD.
    pub text: String,
    /// Bytes the firmware wrote, kept or not.
    pub bytes: u64,
    pub dropped_bytes: u64,
    /// The writes that kept at least one byte, in time order.
    pub writes: Vec<FlexConsoleWrite>,
}

pub(crate) struct Console {
    name: String,
    kept: Vec<u8>,
    bytes: u64,
    writes: Vec<FlexConsoleWrite>,
}

impl Console {
    pub(crate) fn new(config: &FlexConsole) -> Self {
        Self {
            name: config.name.clone(),
            kept: Vec::new(),
            bytes: 0,
            writes: Vec::new(),
        }
    }

    pub(crate) fn summary(&self) -> FlexConsoleSummary {
        FlexConsoleSummary {
            name: self.name.clone(),
            not_modeled: NOT_MODELED,
        }
    }

    pub(crate) fn write(&mut self, time_ns: u64, bytes: &[u8]) {
        self.bytes += bytes.len() as u64;
        let len = bytes.len().min(FLEX_CONSOLE_MAX_BYTES - self.kept.len());
        if len == 0 {
            return;
        }
        self.writes.push(FlexConsoleWrite {
            time_ns,
            offset: self.kept.len(),
            len,
        });
        self.kept.extend_from_slice(&bytes[..len]);
    }

    pub(crate) fn capture(&self) -> FlexConsoleCapture {
        FlexConsoleCapture {
            name: self.name.clone(),
            time_unit: "ns",
            text: String::from_utf8_lossy(&self.kept).into_owned(),
            bytes: self.bytes,
            dropped_bytes: self.bytes - self.kept.len() as u64,
            writes: self.writes.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console() -> Console {
        Console::new(&FlexConsole {
            name: "usb_serial".to_owned(),
        })
    }

    #[test]
    fn writes_join_into_one_text_and_each_keeps_its_time() {
        let mut console = console();
        console.write(10, b"T=2");
        console.write(10, b"");
        console.write(25, b"7.00 C\n");
        let capture = console.capture();
        assert_eq!(capture.text, "T=27.00 C\n");
        assert_eq!(capture.bytes, 10);
        assert_eq!(capture.dropped_bytes, 0);
        assert_eq!(
            capture.writes,
            vec![
                FlexConsoleWrite {
                    time_ns: 10,
                    offset: 0,
                    len: 3
                },
                FlexConsoleWrite {
                    time_ns: 25,
                    offset: 3,
                    len: 7
                },
            ]
        );
    }

    #[test]
    fn bytes_past_the_limit_are_counted_not_kept() {
        let mut console = console();
        console.write(0, &vec![b'a'; FLEX_CONSOLE_MAX_BYTES - 2]);
        console.write(5, b"xyz!");
        console.write(6, b"late");
        let capture = console.capture();
        assert_eq!(capture.bytes, FLEX_CONSOLE_MAX_BYTES as u64 + 6);
        assert_eq!(capture.dropped_bytes, 6);
        assert_eq!(capture.writes.len(), 2);
        assert!(capture.text.ends_with("aaxy"));
    }
}
