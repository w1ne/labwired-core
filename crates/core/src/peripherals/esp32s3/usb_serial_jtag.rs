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
//! |  0x08  | INT_RAW           | R         | SOF bit (0) raised every simulated 1 ms |
//! |  0x0C  | INT_ST            | R         | mirrors INT_RAW |
//! |  0x10  | INT_ENA           | R/W       | stub: 0 (no IRQs) |
//! |  0x14  | INT_CLR           | W         | clears the SOF bit |
//!
//! Plan 2 does not generate interrupts — esp-hal's println path is
//! polling-based. The SOF pulse is not an IRQ: it is the host keepalive
//! Arduino's HWCDC watches. `usb_serial_jtag_sof_tick_hook` reads
//! `int_raw.sof_int_raw` on every FreeRTOS tick and marks the CDC
//! disconnected after ~5 ms without one; while disconnected, `HWCDC::write`
//! drops the bytes instead of queueing them, so a firmware that printed
//! happily on hardware produced an empty capture here (S3 battery run).

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

/// S3 CPU clock — the SOF period is derived from it, so the keepalive is
/// 1 ms of SIMULATED time regardless of wall-clock speed.
const CPU_CLOCK_HZ: u64 = 240_000_000;
const SOF_PERIOD_CYCLES: u64 = CPU_CLOCK_HZ / 1000;

pub struct UsbSerialJtag {
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
    /// Cycles accumulated since the last simulated SOF pulse.
    sof_accum: u64,
    /// A SOF pulse has arrived and not been cleared through INT_CLR.
    sof_pending: bool,
}

impl Default for UsbSerialJtag {
    fn default() -> Self {
        Self::new()
    }
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
            sof_accum: 0,
            sof_pending: false,
        }
    }

    /// Set or clear the byte capture sink and stdout-echo flag.
    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }
}

impl Peripheral for UsbSerialJtag {
    // Walk-active for the SOF keepalive (see `tick_elapsed`): Arduino's HWCDC
    // drops output when no SOF arrives, so the model must tick even though the
    // byte sink itself is polling-based (EP1_CONF always ready, no IRQs).
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            // EP1_CONF (4 bytes, LE): always returns 0x0000_0003
            //   (WR_DONE | SERIAL_IN_EP_DATA_FREE).
            0x04 => Ok(0x03),
            0x05..=0x07 => Ok(0x00),
            // INT_RAW / INT_ST bit 0: the host SOF keepalive.
            0x08 | 0x0C => Ok(u8::from(self.sof_pending)),
            // Remaining INT_* bytes / registers: no other sources.
            _ => Ok(0),
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // EP1 (offset 0x00): only the low byte of the LE word is the data
        // byte; other 3 bytes of a 32-bit write are control bits we ignore.
        if offset == 0x00 {
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
        // INT_CLR (0x14) bit 0 clears the SOF latch, exactly as the driver's
        // `usb_serial_jtag_ll_clr_intsts_mask(SOF)` does after reading it.
        if offset == 0x14 && value & 0x01 != 0 {
            self.sof_pending = false;
        }
        Ok(())
    }

    /// A real full-speed host sends a SOF packet every 1 ms. Arduino's HWCDC
    /// tick hook treats ~5 ms without one as "unplugged" and DROPS every write
    /// instead of queueing it — so the keepalive is what makes a `Serial`
    /// sketch printable on the twin.
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.sof_accum += cycles;
        if self.sof_accum >= SOF_PERIOD_CYCLES {
            self.sof_accum %= SOF_PERIOD_CYCLES;
            self.sof_pending = true;
        }
        PeripheralTickResult::default()
    }

    fn legacy_tick_active(&self) -> bool {
        true
    }

    fn needs_legacy_walk(&self) -> bool {
        true
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
    use crate::bus::SystemBus;
    use crate::Bus;

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
