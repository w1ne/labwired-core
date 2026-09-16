// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! SEGGER RTT (Real-Time Transfer) host-side model.
//!
//! Firmware links the stock `SEGGER_RTT.c`, which keeps a control block and
//! ring buffers in RAM. A real debug probe drains those buffers over SWD while
//! the CPU runs; this model does the same through the emulated bus. It is a
//! pseudo-peripheral: it owns no firmware-visible registers (the sentinel base
//! is never addressed by firmware), it only reads/writes emulated RAM.

use std::any::Any;
use std::sync::{Arc, Mutex};

use crate::{Bus, Peripheral, SimResult};

/// `"SEGGER RTT"` followed by six NULs, at control-block offset 0.
const RTT_ID: [u8; 16] = *b"SEGGER RTT\0\0\0\0\0\0";
const CB_OFF_MAX_UP: u64 = 0x10;
const CB_OFF_AUP0: u64 = 0x18;
const CHAN_SIZE: u64 = 24;
const CHAN_OFF_PBUFFER: u64 = 0x04;
const CHAN_OFF_SIZE: u64 = 0x08;
const CHAN_OFF_WR: u64 = 0x0C;
const CHAN_OFF_RD: u64 = 0x10;
const MAX_CHANNELS: usize = 16;
const DEFAULT_POLL_EVERY_TICKS: u64 = 64;

/// Final-state RTT diagnostics, surfaced in `result.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct RttStatus {
    pub control_block_found: bool,
    pub bytes_drained: u64,
}

#[derive(Debug)]
pub struct SeggerRtt {
    control_block: Option<u32>,
    scan_ranges: Vec<(u64, u64)>,
    found: bool,
    max_up: usize,
    tick_calls: u64,
    poll_every: u64,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
    bytes_drained: u64,
}

impl SeggerRtt {
    pub fn new(control_block: Option<u32>, scan_ranges: Vec<(u64, u64)>) -> Self {
        Self {
            control_block,
            scan_ranges,
            found: false,
            max_up: 0,
            tick_calls: 0,
            poll_every: DEFAULT_POLL_EVERY_TICKS,
            sink: None,
            echo_stdout: false,
            bytes_drained: 0,
        }
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    pub fn status(&self) -> RttStatus {
        RttStatus {
            control_block_found: self.found,
            bytes_drained: self.bytes_drained,
        }
    }

    /// Poll every `n` bus ticks. Default 64: RTT output is human-speed and the
    /// ring buffer is far larger than 64 cycles of firmware writes.
    pub fn set_poll_every_ticks(&mut self, n: u64) {
        self.poll_every = n.max(1);
    }

    fn magic_at(bus: &dyn Bus, addr: u64) -> bool {
        let mut id = [0u8; 16];
        for (i, slot) in id.iter_mut().enumerate() {
            match bus.read_u8(addr + i as u64) {
                Ok(b) => *slot = b,
                Err(_) => return false,
            }
        }
        id == RTT_ID
    }

    fn in_ranges(&self, addr: u64, len: u64) -> bool {
        self.scan_ranges
            .iter()
            .any(|&(base, size)| addr >= base && addr.saturating_add(len) <= base + size)
    }

    fn scan(&self, bus: &dyn Bus) -> Option<u32> {
        for &(base, size) in &self.scan_ranges {
            let mut addr = base;
            while addr + 16 <= base + size {
                if Self::magic_at(bus, addr) {
                    return Some(addr as u32);
                }
                addr += 4;
            }
        }
        None
    }

    fn discover(&mut self, bus: &dyn Bus) {
        if let Some(addr) = self.control_block {
            if Self::magic_at(bus, addr as u64) {
                self.found = true;
            }
        } else if let Some(addr) = self.scan(bus) {
            self.control_block = Some(addr);
            self.found = true;
        }
        if self.found {
            self.max_up = bus
                .read_u32(self.control_block.unwrap() as u64 + CB_OFF_MAX_UP)
                .unwrap_or(0)
                .min(MAX_CHANNELS as u32) as usize;
        }
    }

    fn drain(&mut self, bus: &mut dyn Bus) {
        let Some(cb) = self.control_block else { return };
        for i in 0..self.max_up {
            let desc = cb as u64 + CB_OFF_AUP0 + CHAN_SIZE * i as u64;
            let (Ok(p_buffer), Ok(size), Ok(wr), Ok(rd)) = (
                bus.read_u32(desc + CHAN_OFF_PBUFFER),
                bus.read_u32(desc + CHAN_OFF_SIZE),
                bus.read_u32(desc + CHAN_OFF_WR),
                bus.read_u32(desc + CHAN_OFF_RD),
            ) else {
                continue;
            };
            if size < 2 || p_buffer == 0 {
                continue;
            }
            if !self.in_ranges(p_buffer as u64, size as u64) {
                continue;
            }
            if rd >= size || wr >= size || wr == rd {
                continue;
            }
            let len = if wr > rd { wr - rd } else { size - rd + wr };
            let mut buf = Vec::with_capacity(len as usize);
            for k in 0..len {
                let idx = (rd as u64 + k as u64) % size as u64;
                match bus.read_u8(p_buffer as u64 + idx) {
                    Ok(b) => buf.push(b),
                    Err(_) => {
                        buf.clear();
                        break;
                    }
                }
            }
            if buf.is_empty() {
                continue;
            }
            if let Some(sink) = &self.sink {
                if let Ok(mut g) = sink.lock() {
                    g.extend_from_slice(&buf);
                }
            }
            if self.echo_stdout {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = out.write_all(&buf);
                let _ = out.flush();
            }
            self.bytes_drained += buf.len() as u64;
            let _ = bus.write_u32(desc + CHAN_OFF_RD, wr);
        }
    }
}

impl Peripheral for SeggerRtt {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.tick_calls = self.tick_calls.wrapping_add(1);
        if self.tick_calls % self.poll_every != 0 {
            return;
        }
        if !self.found {
            self.discover(bus);
            if !self.found {
                return;
            }
        }
        self.drain(bus);
    }

    fn needs_bus_tick(&self) -> bool {
        true
    }

    fn idle_poll_bus_tick(&self) -> bool {
        true
    }

    /// All work happens in `tick_with_bus`; `tick()` is a structural no-op.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    /// Honest per the `Peripheral::needs_legacy_walk` contract: deleting the
    /// legacy walk cannot change RTT output because the bus-tick pass runs
    /// independently of walk deletion.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;

    fn write_id(bus: &mut SystemBus, cb: u64) {
        for (i, b) in RTT_ID.iter().enumerate() {
            bus.ram.write_u8(cb + i as u64, *b);
        }
    }

    fn write_u32_at(bus: &mut SystemBus, addr: u64, v: u32) {
        assert!(bus.ram.write_u32(addr, v), "RAM write at {addr:#x}");
    }

    fn setup_cb(bus: &mut SystemBus, cb: u64, buf: u64, size: u32, wr: u32, rd: u32) {
        write_id(bus, cb);
        write_u32_at(bus, cb + CB_OFF_MAX_UP, 1);
        let desc = cb + CB_OFF_AUP0;
        write_u32_at(bus, desc + CHAN_OFF_PBUFFER, buf as u32);
        write_u32_at(bus, desc + CHAN_OFF_SIZE, size);
        write_u32_at(bus, desc + CHAN_OFF_WR, wr);
        write_u32_at(bus, desc + CHAN_OFF_RD, rd);
    }

    #[test]
    fn drains_linear_ring_and_advances_rd() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let buf = 0x2000_1000u64;
        setup_cb(&mut bus, cb, buf, 16, 3, 0);
        for (i, b) in b"hi!".iter().enumerate() {
            bus.ram.write_u8(buf + i as u64, *b);
        }

        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_ticks(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));

        bus.tick_peripherals_with_costs();

        assert_eq!(&*sink.lock().unwrap(), b"hi!");
        assert_eq!(bus.ram.read_u32(cb + CB_OFF_AUP0 + CHAN_OFF_RD), Some(3));
        let status = bus.segger_rtt_status().expect("rtt status");
        assert!(status.control_block_found);
        assert_eq!(status.bytes_drained, 3);
    }

    #[test]
    fn drains_wrapped_ring_once() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let buf = 0x2000_1000u64;
        setup_cb(&mut bus, cb, buf, 8, 3, 6);
        for (i, b) in b"hello".iter().enumerate() {
            let idx = (6 + i) % 8;
            bus.ram.write_u8(buf + idx as u64, *b);
        }

        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_ticks(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));
        bus.tick_peripherals_with_costs();

        assert_eq!(&*sink.lock().unwrap(), b"hello");
        assert_eq!(bus.ram.read_u32(cb + CB_OFF_AUP0 + CHAN_OFF_RD), Some(3));
    }

    #[test]
    fn zero_initialized_cb_is_not_fatal_and_later_init_is_drained() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let buf = 0x2000_1000u64;

        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_ticks(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));

        bus.tick_peripherals_with_costs();
        assert!(sink.lock().unwrap().is_empty());
        assert!(!bus.segger_rtt_status().unwrap().control_block_found);

        setup_cb(&mut bus, cb, buf, 8, 2, 0);
        bus.ram.write_u8(buf, b'O');
        bus.ram.write_u8(buf + 1, b'K');
        bus.tick_peripherals_with_costs();

        assert_eq!(&*sink.lock().unwrap(), b"OK");
        assert!(bus.segger_rtt_status().unwrap().control_block_found);
    }

    #[test]
    fn scan_finds_magic_when_no_address_is_supplied() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_2000u64;
        let buf = 0x2000_3000u64;
        setup_cb(&mut bus, cb, buf, 8, 1, 0);
        bus.ram.write_u8(buf, b'X');

        let mut rtt = SeggerRtt::new(None, vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_ticks(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));
        bus.tick_peripherals_with_costs();

        assert_eq!(&*sink.lock().unwrap(), b"X");
    }

    #[test]
    fn garbage_channel_pointers_are_skipped() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        setup_cb(&mut bus, cb, 0xDEAD_BEEF, 8, 2, 0);

        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_ticks(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));
        bus.tick_peripherals_with_costs();

        assert!(
            sink.lock().unwrap().is_empty(),
            "out-of-RAM pointer must not drain"
        );
    }
}
