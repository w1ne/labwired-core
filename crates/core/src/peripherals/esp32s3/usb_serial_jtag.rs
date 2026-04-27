// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! USB_SERIAL_JTAG peripheral for ESP32-S3.
//!
//! The S3 exposes a CDC-ACM device over USB that shares the same physical
//! USB cable as the JTAG debug interface.  When a host connects, it sees a
//! `/dev/ttyACM*` device on which the firmware can print.
//!
//! In the simulator we don't model the USB protocol — we expose just the
//! MMIO interface the firmware writes to.  Bytes written to EP1 are
//! appended to a sink (a `Vec<u8>` for tests) and optionally echoed to
//! host stdout for live runs.
//!
//! ## Register layout (ESP32-S3 TRM §27.5)
//!
//! | Offset | Name              | Direction | Behaviour |
//! |-------:|-------------------|-----------|-----------|
//! |  0x00  | EP1               | W         | byte FIFO data; bottom 8 bits of write are appended |
//! |  0x04  | EP1_CONF          | R         | reads `WR_DONE | SERIAL_IN_EP_DATA_FREE = 0x3` always |
//! |  0x08  | INT_RAW           | R/W       | stub: 0 |
//! |  0x0C  | INT_ST            | R         | stub: 0 |
//! |  0x10  | INT_ENA           | R/W       | stub: 0 (no IRQs in Plan 2) |
//! |  0x14  | INT_CLR           | W         | stub: NOP |
//!
//! Plan 2 does not generate interrupts — esp-hal's println path is
//! polling-based.

use crate::{Peripheral, SimResult};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct UsbSerialJtag {
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
}

impl std::fmt::Debug for UsbSerialJtag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UsbSerialJtag(sink={}, echo_stdout={})",
            self.sink.is_some(),
            self.echo_stdout,
        )
    }
}

impl UsbSerialJtag {
    pub fn new() -> Self {
        Self {
            sink: None,
            echo_stdout: true,
        }
    }

    /// Set or clear the byte capture sink and stdout-echo flag.
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }
}

impl Peripheral for UsbSerialJtag {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            // EP1_CONF (4 bytes, LE): always returns 0x0000_0003
            //   (WR_DONE | SERIAL_IN_EP_DATA_FREE).
            0x04 => Ok(0x03),
            0x05..=0x07 => Ok(0x00),
            // INT_* registers stub to 0.
            _ => Ok(0),
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            // EP1: only the low byte of the LE word is the data byte.
            // The other 3 bytes of a 32-bit write are control bits we ignore.
            0x00 => {
                if let Some(sink) = &self.sink {
                    if let Ok(mut g) = sink.lock() {
                        g.push(value);
                    }
                }
                if self.echo_stdout {
                    let _ = io::stdout().write_all(&[value]);
                    let _ = io::stdout().flush();
                }
            }
            // INT_* writes accepted silently.
            _ => {}
        }
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bus;
    use crate::bus::SystemBus;

    #[test]
    fn ep1_conf_reads_constant() {
        let p = UsbSerialJtag::new();
        // 32-bit read at 0x04 = 0x00000003 LE.
        assert_eq!(p.read(0x04).unwrap(), 0x03);
        assert_eq!(p.read(0x05).unwrap(), 0x00);
        assert_eq!(p.read(0x06).unwrap(), 0x00);
        assert_eq!(p.read(0x07).unwrap(), 0x00);
    }

    #[test]
    fn writing_ep1_appends_to_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut p = UsbSerialJtag::new();
        p.set_sink(Some(sink.clone()), false);
        p.write(0x00, b'H').unwrap();
        p.write(0x00, b'i').unwrap();
        assert_eq!(sink.lock().unwrap().as_slice(), b"Hi");
    }

    #[test]
    fn writing_via_bus_word_write_appends_low_byte() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut bus = SystemBus::new();
        let mut p = UsbSerialJtag::new();
        p.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("usb_jtag", 0x6003_8000, 0x100, None, Box::new(p));

        // Simulate `sw a2, 0(a1)` writing 'H' = 0x48 to the FIFO.
        bus.write_u32(0x6003_8000, 0x0000_0048).unwrap();
        // The write_u32 path decomposes into 4 byte writes at offsets 0..=3.
        // Offset 0 (low byte) is 'H'; the 3 high bytes go to offsets 1..=3,
        // which are not the FIFO byte — they're silently accepted.
        assert_eq!(sink.lock().unwrap().as_slice(), b"H");
    }

    #[test]
    fn int_registers_stub_to_zero() {
        let p = UsbSerialJtag::new();
        for off in 0x08..=0x17u64 {
            assert_eq!(p.read(off).unwrap(), 0, "offset 0x{off:02x}");
        }
    }
}
