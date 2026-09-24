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
//!
//! Draining is paced in simulated CPU cycles (default 64), not bus-tick calls,
//! so a blocking `BLOCK_IF_FIFO_FULL` write is released even when the Cortex-M
//! JIT widens the tick interval. The no-address magic scan is separately
//! throttled and cursor-resumable, so a no-hit 1 MiB sweep is spread across
//! polls instead of restarting from the first range base every time.

use std::any::Any;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::{Bus, CycleClock, Peripheral, SimResult};

/// `"SEGGER RTT"` followed by six NULs, at control-block offset 0.
const RTT_ID: [u8; 16] = *b"SEGGER RTT\0\0\0\0\0\0";
const CB_OFF_MAX_UP: u64 = 0x10;
const CB_OFF_MAX_DOWN: u64 = 0x14;
const CB_OFF_AUP0: u64 = 0x18;
const CHAN_SIZE: u64 = 24;
const CHAN_OFF_PBUFFER: u64 = 0x04;
const CHAN_OFF_SIZE: u64 = 0x08;
const CHAN_OFF_WR: u64 = 0x0C;
const CHAN_OFF_RD: u64 = 0x10;
const CHAN_OFF_FLAGS: u64 = 0x14;
const MAX_CHANNELS: usize = 16;
const DEFAULT_POLL_EVERY_CYCLES: u64 = 64;
const DEFAULT_SCAN_EVERY_POLLS: u64 = 64;
/// Candidates probed per `scan` call. A 1 MiB RAM holds 262,144 candidates, so
/// one poll cannot sweep it; the cursor resumes where the last poll stopped.
const SCAN_CANDIDATES_PER_POLL: u32 = 4096;

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
    /// Resume point for the bounded magic sweep: `(range index, byte offset)`.
    scan_cursor: (usize, u64),
    found: bool,
    /// Cycle-cadence polls run since attach; drives scan throttling.
    polls: u64,
    scan_every_polls: u64,
    poll_every_cycles: u64,
    clock: Option<CycleClock>,
    last_poll_cycle: Option<u64>,
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
    bytes_drained: u64,
    /// Host bytes waiting for down-channel 0. The probe writes `WrOff`; the
    /// target's `SEGGER_RTT_GetKey` / `SEGGER_RTT_Read` advances `RdOff`.
    input: Mutex<VecDeque<u8>>,
}

impl SeggerRtt {
    pub fn new(control_block: Option<u32>, scan_ranges: Vec<(u64, u64)>) -> Self {
        Self {
            control_block,
            scan_ranges,
            scan_cursor: (0, 0),
            found: false,
            polls: 0,
            scan_every_polls: DEFAULT_SCAN_EVERY_POLLS,
            poll_every_cycles: DEFAULT_POLL_EVERY_CYCLES,
            clock: None,
            last_poll_cycle: None,
            sink: None,
            echo_stdout: false,
            bytes_drained: 0,
            input: Mutex::new(VecDeque::new()),
        }
    }

    /// Queue bytes for down-channel 0. They land in the target ring on the
    /// next probe poll, stopping one byte short of `RdOff` so a full buffer
    /// cannot look empty. Leftovers stay queued until the target reads.
    pub fn queue_input(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if let Ok(mut q) = self.input.lock() {
            q.extend(data);
        }
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }

    /// Take everything the capture sink has accumulated since the last call.
    /// Empty when no sink is attached. Never touches the ring cursors — the
    /// probe-side protocol is unaffected by who consumes the bytes.
    pub fn drain_captured(&self) -> Vec<u8> {
        let Some(sink) = &self.sink else {
            return Vec::new();
        };
        match sink.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(_) => Vec::new(),
        }
    }

    pub fn status(&self) -> RttStatus {
        RttStatus {
            control_block_found: self.found,
            bytes_drained: self.bytes_drained,
        }
    }

    /// Poll every `n` simulated CPU cycles. Default 64: the cadence is bounded
    /// in cycles regardless of the bus's `peripheral_tick_interval`/JIT window
    /// length, so a blocking `BLOCK_IF_FIFO_FULL` write never spins for a whole
    /// window — the probe catches up at the next 64-cycle boundary.
    pub fn set_poll_every_cycles(&mut self, n: u64) {
        self.poll_every_cycles = n.max(1);
    }

    /// Run the bounded RAM magic scan on at most every `n`th poll. Default 64:
    /// a full sweep already spans polls via the cursor, so probing more often
    /// only burns reads. The first poll always scans, and an explicit
    /// `control_block` (the ELF-symbol path) is checked every due poll
    /// regardless — only the sweep is throttled.
    pub fn set_scan_every_polls(&mut self, n: u64) {
        self.scan_every_polls = n.max(1);
    }

    /// True when `poll_every_cycles` have elapsed since the last poll. A
    /// hand-built bus that never called [`Peripheral::attach_cycle_clock`] has
    /// no clock, so every call polls (historical behaviour); with a clock, the
    /// first call polls immediately and later calls wait out the cycle budget.
    fn poll_due(&self) -> bool {
        match (&self.clock, self.last_poll_cycle) {
            (Some(clock), Some(last)) => clock.now().saturating_sub(last) >= self.poll_every_cycles,
            _ => true,
        }
    }

    /// True when this poll should run the bounded magic scan. `polls` is
    /// incremented before this is called, so poll 1 scans immediately and
    /// later scans land every `scan_every_polls` polls.
    fn scan_due(&self) -> bool {
        (self.polls - 1) % self.scan_every_polls == 0
    }

    /// True when the 16-byte RTT ID sits at `addr`. Bails on the first
    /// mismatching byte, so a full no-hit sweep costs one read per candidate
    /// rather than sixteen.
    fn magic_at(bus: &dyn Bus, addr: u64) -> bool {
        for (i, expected) in RTT_ID.iter().enumerate() {
            match bus.read_u8(addr + i as u64) {
                Ok(b) if b == *expected => {}
                _ => return false,
            }
        }
        true
    }

    fn in_ranges(&self, addr: u64, len: u64) -> bool {
        self.scan_ranges
            .iter()
            .any(|&(base, size)| addr >= base && addr.saturating_add(len) <= base + size)
    }

    /// Bounded magic sweep over the configured ranges. It resumes from
    /// `scan_cursor` and probes at most `SCAN_CANDIDATES_PER_POLL` candidates
    /// per call, so a 1 MiB RAM is swept across polls instead of restarting —
    /// and re-reading — from the first range base every time.
    fn scan(&mut self, bus: &dyn Bus) -> Option<u32> {
        let n = self.scan_ranges.len();
        if n == 0 {
            self.scan_cursor = (0, 0);
            return None;
        }
        let mut budget = SCAN_CANDIDATES_PER_POLL;
        let (mut range_idx, mut offset) = self.scan_cursor;
        if range_idx >= n {
            range_idx = 0;
            offset = 0;
        }
        let mut skipped = 0usize;
        while skipped < n {
            let (base, size) = self.scan_ranges[range_idx];
            if offset + 16 > size {
                range_idx = (range_idx + 1) % n;
                offset = 0;
                skipped += 1;
                continue;
            }
            skipped = 0;
            if budget == 0 {
                self.scan_cursor = (range_idx, offset);
                return None;
            }
            if Self::magic_at(bus, base + offset) {
                self.scan_cursor = (range_idx, offset + 4);
                return Some((base + offset) as u32);
            }
            offset += 4;
            budget -= 1;
        }
        // Every range is smaller than the ID: nothing can ever match.
        self.scan_cursor = (0, 0);
        None
    }

    fn discover(&mut self, bus: &dyn Bus) {
        if let Some(addr) = self.control_block {
            // Range-guard the ELF-supplied address: a control block can only
            // live in RAM, and probing a stale symbol that resolved into MMIO
            // would read registers with read-to-clear side effects.
            if self.in_ranges(addr as u64, 16) && Self::magic_at(bus, addr as u64) {
                self.found = true;
            }
        } else if let Some(addr) = self.scan(bus) {
            self.control_block = Some(addr);
            self.found = true;
        }
    }

    fn drain(&mut self, bus: &mut dyn Bus) {
        let Some(cb) = self.control_block else { return };
        let cb = cb as u64;
        // Re-read the count every poll rather than latching it at discovery:
        // the stock `_DoInit` writes it before the ID, but a re-init or a
        // partially-written control block must recover, not stall silently
        // behind a stale zero.
        let max_up = bus
            .read_u32(cb + CB_OFF_MAX_UP)
            .unwrap_or(0)
            .min(MAX_CHANNELS as u32) as usize;
        for i in 0..max_up {
            let desc = cb + CB_OFF_AUP0 + CHAN_SIZE * i as u64;
            let (Ok(p_buffer), Ok(size), Ok(wr), Ok(rd), Ok(flags)) = (
                bus.read_u32(desc + CHAN_OFF_PBUFFER),
                bus.read_u32(desc + CHAN_OFF_SIZE),
                bus.read_u32(desc + CHAN_OFF_WR),
                bus.read_u32(desc + CHAN_OFF_RD),
                bus.read_u32(desc + CHAN_OFF_FLAGS),
            ) else {
                continue;
            };
            if size < 2 || p_buffer == 0 {
                continue;
            }
            // SEGGER requires the upper Flags byte (Flags[31:24]) to be zero
            // on stock channels as a validity check; nonzero means this is not
            // a stock channel and draining it would be wrong.
            if flags >> 24 != 0 {
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
            // Counted even when no sink/echo is attached: the bytes still left
            // the firmware's ring (all current callers attach one; a standalone
            // `attach_segger_rtt` discards them).
            self.bytes_drained += buf.len() as u64;
            let _ = bus.write_u32(desc + CHAN_OFF_RD, wr);
        }
    }

    /// `aDown` starts at `aUp[MaxNumUpBuffers]`. A zero up-count, or a control
    /// block whose ID is not present yet, must not produce that address: it
    /// would alias `aDown` onto `aUp`. Re-read every call; `_DoInit` publishes
    /// the count before the ID, and a later re-init can change it.
    fn down_channel_desc(bus: &dyn Bus, cb: u64, index: u32) -> Option<u64> {
        if !Self::magic_at(bus, cb) {
            return None;
        }
        let max_up = bus.read_u32(cb + CB_OFF_MAX_UP).unwrap_or(0);
        if max_up == 0 {
            return None;
        }
        let max_down = bus.read_u32(cb + CB_OFF_MAX_DOWN).unwrap_or(0);
        if index >= max_down {
            return None;
        }
        Some(cb + CB_OFF_AUP0 + CHAN_SIZE * u64::from(max_up) + CHAN_SIZE * u64::from(index))
    }

    /// `None` when this descriptor must not be written: bad geometry, a
    /// non-zero `Flags[31:24]`, or a buffer outside the RAM `attach_segger_rtt`
    /// already collected.
    fn usable_down(&self, bus: &dyn Bus, desc: u64) -> Option<(u32, u32, u32, u32)> {
        let (Ok(p_buffer), Ok(size), Ok(wr), Ok(rd), Ok(flags)) = (
            bus.read_u32(desc + CHAN_OFF_PBUFFER),
            bus.read_u32(desc + CHAN_OFF_SIZE),
            bus.read_u32(desc + CHAN_OFF_WR),
            bus.read_u32(desc + CHAN_OFF_RD),
            bus.read_u32(desc + CHAN_OFF_FLAGS),
        ) else {
            return None;
        };
        if size < 2 || p_buffer == 0 || flags >> 24 != 0 || rd >= size || wr >= size {
            return None;
        }
        if !self.in_ranges(p_buffer as u64, size as u64) {
            return None;
        }
        Some((p_buffer, size, wr, rd))
    }

    /// Free bytes in a down ring. One slot stays empty so full and empty stay
    /// distinct: zero means full.
    fn free_down(size: u32, wr: u32, rd: u32) -> u32 {
        if rd > wr {
            rd - wr - 1
        } else {
            size - (wr - rd + 1)
        }
    }

    /// Copy `data` into down-channel `index` and store the new `WrOff` once.
    /// Returns how many bytes fit. The rest is discarded; this does not stall
    /// and does not write `RdOff`. No control block, a missing ID, or
    /// `MaxNumUpBuffers == 0` accepts nothing.
    pub fn write_down(&self, bus: &mut dyn Bus, index: u32, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0;
        }
        let Some(cb) = self.control_block else {
            return 0;
        };
        let Some(desc) = Self::down_channel_desc(bus, cb as u64, index) else {
            return 0;
        };
        let Some((p_buffer, size, mut wr, rd)) = self.usable_down(bus, desc) else {
            return 0;
        };
        let n = (Self::free_down(size, wr, rd) as usize).min(data.len());
        let mut accepted = 0usize;
        for &byte in &data[..n] {
            if bus.write_u8(p_buffer as u64 + u64::from(wr), byte).is_err() {
                break;
            }
            wr = if wr + 1 == size { 0 } else { wr + 1 };
            accepted += 1;
        }
        if accepted > 0 && bus.write_u32(desc + CHAN_OFF_WR, wr).is_err() {
            return 0;
        }
        accepted
    }

    /// Queued host bytes for down-channel 0. `write_rtt_input` has no bus
    /// borrow, so the copy waits for this poll. [`Self::write_down`] stores
    /// immediately and does not use this queue.
    fn fill_down(&mut self, bus: &mut dyn Bus) {
        let Some(cb) = self.control_block else { return };
        let Some(desc) = Self::down_channel_desc(bus, cb as u64, 0) else {
            return;
        };
        let Some((p_buffer, size, mut wr, rd)) = self.usable_down(bus, desc) else {
            return;
        };
        let Ok(mut q) = self.input.lock() else { return };
        if q.is_empty() {
            return;
        }
        let mut wrote = false;
        while !q.is_empty() {
            if Self::free_down(size, wr, rd) == 0 {
                break;
            }
            let Some(byte) = q.pop_front() else { break };
            if bus.write_u8(p_buffer as u64 + u64::from(wr), byte).is_err() {
                q.push_front(byte);
                break;
            }
            wr = if wr + 1 == size { 0 } else { wr + 1 };
            wrote = true;
        }
        if wrote {
            let _ = bus.write_u32(desc + CHAN_OFF_WR, wr);
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

    /// The bus hands every `add_peripheral`-attached model the shared cycle
    /// clock; the RTT probe keys its drain cadence on it.
    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if !self.poll_due() {
            return;
        }
        self.polls = self.polls.saturating_add(1);
        self.last_poll_cycle = self.clock.as_ref().map(|c| c.now());
        if !self.found {
            // An explicit `control_block` check is a single 16-byte magic read,
            // so it re-tries on every due drain poll and notices a late
            // `_DoInit` within `poll_every_cycles`; only the RAM sweep is
            // throttled, because a full sweep already spans polls via the
            // cursor and probing more often only burns reads.
            if self.control_block.is_none() && !self.scan_due() {
                return;
            }
            self.discover(bus);
            if !self.found {
                return;
            }
        }
        self.drain(bus);
        self.fill_down(bus);
    }

    fn needs_bus_tick(&self) -> bool {
        true
    }

    /// Deliberate: `run --rtt` text should surface promptly even while the CPU
    /// idle-fasts-forward, and the bounded idle-skip cap the machine pays for
    /// it is small. Not a correctness need — the ring survives any skip.
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
        rtt.set_poll_every_cycles(1);
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
    fn second_poll_does_not_redrain_the_same_bytes() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let buf = 0x2000_1000u64;
        setup_cb(&mut bus, cb, buf, 16, 3, 0);
        for (i, b) in b"hi!".iter().enumerate() {
            bus.ram.write_u8(buf + i as u64, *b);
        }

        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_cycles(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));

        bus.tick_peripherals_with_costs();
        assert_eq!(&*sink.lock().unwrap(), b"hi!");

        // RdOff was advanced to WrOff, so the next poll sees wr == rd. Only the
        // RdOff write-back stops the same three bytes being appended again.
        // The cadence is cycle-bounded, so the clock must advance to re-poll.
        bus.set_current_cycle(100);
        bus.tick_peripherals_with_costs();
        assert_eq!(&*sink.lock().unwrap(), b"hi!");
        assert_eq!(bus.segger_rtt_status().unwrap().bytes_drained, 3);
    }

    #[test]
    fn drain_captured_takes_bytes_once_and_survives_no_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut rtt = SeggerRtt::new(None, vec![]);
        rtt.set_sink(Some(sink.clone()), false);
        sink.lock().unwrap().extend_from_slice(b"hello rtt");

        assert_eq!(rtt.drain_captured(), b"hello rtt");
        assert_eq!(rtt.drain_captured(), b"");

        let no_sink = SeggerRtt::new(None, vec![]);
        assert_eq!(no_sink.drain_captured(), b"");
    }

    #[test]
    fn drain_captured_returns_empty_on_a_poisoned_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let mut rtt = SeggerRtt::new(None, vec![]);
        rtt.set_sink(Some(sink.clone()), false);
        let poisoner = sink.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the sink");
        })
        .join();
        assert!(rtt.drain_captured().is_empty());
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
        rtt.set_poll_every_cycles(1);
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
        rtt.set_poll_every_cycles(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));

        bus.tick_peripherals_with_costs();
        assert!(sink.lock().unwrap().is_empty());
        assert!(!bus.segger_rtt_status().unwrap().control_block_found);

        setup_cb(&mut bus, cb, buf, 8, 2, 0);
        bus.ram.write_u8(buf, b'O');
        bus.ram.write_u8(buf + 1, b'K');
        bus.set_current_cycle(100);
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
        rtt.set_poll_every_cycles(1);
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
        // Mapped, but NOT RAM: `SystemBus::new()` registers a UART at
        // 0x4000_C000. Without the `in_ranges` guard this drains UART
        // registers into the sink (and can read-to-clear them), so removing
        // the guard fails this test.
        setup_cb(&mut bus, cb, 0x4000_C000, 8, 2, 0);

        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_cycles(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));
        bus.tick_peripherals_with_costs();

        assert!(
            sink.lock().unwrap().is_empty(),
            "non-RAM pointer must not drain"
        );
    }

    #[test]
    fn hostile_channel_geometry_and_flags_are_skipped() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let buf = 0x2000_1000u64;

        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_cycles(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));

        // WrOff must stay below SizeOfBuffer.
        setup_cb(&mut bus, cb, buf, 8, 8, 0);
        bus.tick_peripherals_with_costs();
        assert!(
            sink.lock().unwrap().is_empty(),
            "wr >= size must be skipped"
        );

        // SizeOfBuffer must hold at least two bytes. The cadence is
        // cycle-bounded, so advance the clock to re-poll.
        setup_cb(&mut bus, cb, buf, 1, 0, 0);
        bus.set_current_cycle(10);
        bus.tick_peripherals_with_costs();
        assert!(sink.lock().unwrap().is_empty(), "size < 2 must be skipped");

        // SEGGER reserves the upper Flags byte; nonzero marks a non-stock
        // channel that must not be drained.
        setup_cb(&mut bus, cb, buf, 8, 2, 0);
        write_u32_at(&mut bus, cb + CB_OFF_AUP0 + CHAN_OFF_FLAGS, 1 << 24);
        bus.ram.write_u8(buf, b'N');
        bus.set_current_cycle(20);
        bus.tick_peripherals_with_costs();
        assert!(
            sink.lock().unwrap().is_empty(),
            "nonzero upper Flags byte must be skipped"
        );
    }

    #[test]
    fn bounded_scan_resumes_and_finds_cb_near_end_of_ram() {
        let mut bus = SystemBus::new();
        let cb = 0x200F_F000u64;
        let buf = 0x200E_1000u64;
        setup_cb(&mut bus, cb, buf, 8, 1, 0);
        bus.ram.write_u8(buf, b'Z');

        let mut rtt = SeggerRtt::new(None, vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_cycles(1);
        rtt.set_scan_every_polls(1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));

        // 0x200F_F000 is 261,120 candidates past the scan base and the capped
        // sweep probes 4,096 per poll, so one tick cannot reach it. Repeated
        // ticks must resume the cursor until the CB is found and drained. The
        // cadence is cycle-bounded, so the clock must advance between polls.
        bus.tick_peripherals_with_costs();
        assert!(
            sink.lock().unwrap().is_empty(),
            "one capped poll cannot sweep 1 MiB"
        );
        assert!(!bus.segger_rtt_status().unwrap().control_block_found);

        for cycle in 1..=256u64 {
            bus.set_current_cycle(cycle);
            bus.tick_peripherals_with_costs();
        }
        assert_eq!(&*sink.lock().unwrap(), b"Z");
        assert!(bus.segger_rtt_status().unwrap().control_block_found);
    }

    #[test]
    fn poll_cadence_is_cycle_bounded() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let buf = 0x2000_1000u64;
        setup_cb(&mut bus, cb, buf, 16, 2, 0);
        bus.ram.write_u8(buf, b'a');
        bus.ram.write_u8(buf + 1, b'b');

        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_cycles(64);
        let sink = Arc::new(Mutex::new(Vec::new()));
        rtt.set_sink(Some(sink.clone()), false);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));

        // Cycle 0: the first poll is immediate and drains "ab".
        bus.tick_peripherals_with_costs();
        assert_eq!(sink.lock().unwrap().len(), 2);

        // "cd" arrives, but 10 cycles is inside the 64-cycle budget, so the
        // new bytes must stay in the ring.
        bus.ram.write_u8(buf + 2, b'c');
        bus.ram.write_u8(buf + 3, b'd');
        write_u32_at(&mut bus, cb + CB_OFF_AUP0 + CHAN_OFF_WR, 4);
        bus.set_current_cycle(10);
        bus.tick_peripherals_with_costs();
        assert_eq!(
            sink.lock().unwrap().len(),
            2,
            "10 cycles is inside the 64-cycle budget"
        );

        // 100 cycles have elapsed since the last poll, so the drain runs again.
        bus.set_current_cycle(100);
        bus.tick_peripherals_with_costs();
        assert_eq!(&*sink.lock().unwrap(), b"abcd");

        // JIT-window case: the bus-tick pass runs only at window boundaries
        // 512 cycles apart. Each boundary is past the 64-cycle budget, so each
        // one must drain the window's bytes — one drain per boundary, not one
        // per 64 boundaries.
        bus.ram.write_u8(buf + 4, b'e');
        bus.ram.write_u8(buf + 5, b'f');
        write_u32_at(&mut bus, cb + CB_OFF_AUP0 + CHAN_OFF_WR, 6);
        bus.set_current_cycle(612);
        bus.tick_peripherals_with_costs();
        assert_eq!(&*sink.lock().unwrap(), b"abcdef");

        bus.ram.write_u8(buf + 6, b'g');
        bus.ram.write_u8(buf + 7, b'h');
        write_u32_at(&mut bus, cb + CB_OFF_AUP0 + CHAN_OFF_WR, 8);
        bus.set_current_cycle(1124);
        bus.tick_peripherals_with_costs();
        assert_eq!(&*sink.lock().unwrap(), b"abcdefgh");
    }

    /// Down-channel 0 sits after the whole up array. The stock library compiles
    /// three up buffers, so a host that assumes "the slot after up[0]" writes
    /// into the wrong descriptor and `SEGGER_RTT_GetKey` stays empty.
    fn down_desc(cb: u64, max_up: u64) -> u64 {
        cb + CB_OFF_AUP0 + CHAN_SIZE * max_up
    }

    fn setup_down(
        bus: &mut SystemBus,
        cb: u64,
        max_up: u32,
        buf: u64,
        size: u32,
        wr: u32,
        rd: u32,
    ) {
        write_u32_at(bus, cb + CB_OFF_MAX_DOWN, 1);
        write_u32_at(bus, cb + CB_OFF_MAX_UP, max_up);
        let desc = down_desc(cb, max_up as u64);
        write_u32_at(bus, desc + CHAN_OFF_PBUFFER, buf as u32);
        write_u32_at(bus, desc + CHAN_OFF_SIZE, size);
        write_u32_at(bus, desc + CHAN_OFF_WR, wr);
        write_u32_at(bus, desc + CHAN_OFF_RD, rd);
    }

    fn attach(bus: &mut SystemBus, cb: u64) {
        let mut rtt = SeggerRtt::new(Some(cb as u32), vec![(0x2000_0000, 0x10_0000)]);
        rtt.set_poll_every_cycles(1);
        bus.add_peripheral("segger_rtt", 0xE00F_F000, 0x1000, None, Box::new(rtt));
    }

    #[test]
    fn host_write_lands_in_down_channel_0_and_leaves_rd_to_the_target() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let up = 0x2000_1000u64;
        let down = 0x2000_2000u64;
        // Stock SEGGER_RTT_Conf.h: 3 up buffers, then the down array.
        setup_cb(&mut bus, cb, up, 16, 0, 0);
        setup_down(&mut bus, cb, 3, down, 16, 0, 0);
        attach(&mut bus, cb);

        assert!(bus.write_rtt_input(b"ab"));
        bus.tick_peripherals_with_costs();

        assert_eq!(bus.ram.read_u8(down), Some(b'a'));
        assert_eq!(bus.ram.read_u8(down + 1), Some(b'b'));
        let desc = down_desc(cb, 3);
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_WR), Some(2));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_RD), Some(0));
    }

    #[test]
    fn host_write_keeps_one_byte_free_and_resumes_after_the_target_reads() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let up = 0x2000_1000u64;
        let down = 0x2000_2000u64;
        setup_cb(&mut bus, cb, up, 8, 0, 0);
        // Size 4 holds 3 bytes. `WrOff == RdOff` means empty, so the 4th slot stays free.
        setup_down(&mut bus, cb, 3, down, 4, 0, 0);
        attach(&mut bus, cb);

        assert!(bus.write_rtt_input(b"abcde"));
        bus.tick_peripherals_with_costs();
        let desc = down_desc(cb, 3);
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_WR), Some(3));
        assert_eq!(bus.ram.read_u8(down), Some(b'a'));
        assert_eq!(bus.ram.read_u8(down + 2), Some(b'c'));

        // Target consumed a,b,c the way SEGGER_RTT_ReadNoLock advances RdOff.
        write_u32_at(&mut bus, desc + CHAN_OFF_RD, 3);
        bus.set_current_cycle(100);
        bus.tick_peripherals_with_costs();
        assert_eq!(bus.ram.read_u8(down + 3), Some(b'd'));
        assert_eq!(bus.ram.read_u8(down), Some(b'e'));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_WR), Some(1));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_RD), Some(3));
    }

    #[test]
    fn host_write_waits_until_a_down_channel_exists() {
        let bus = SystemBus::new();
        assert!(!bus.write_rtt_input(b"x"));
    }

    /// Wrap, the one-byte gap that means full, and `Flags[31:24] != 0`.
    #[test]
    fn write_down_wraps_stops_when_one_byte_free_and_skips_flagged_channels() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let up = 0x2000_1000u64;
        let down = 0x2000_2000u64;
        setup_cb(&mut bus, cb, up, 16, 0, 0);
        // Size 8, WrOff 6, RdOff 1: two free slots (6, 7). Index 0 is the
        // reserved byte that keeps the ring from looking empty.
        setup_down(&mut bus, cb, 3, down, 8, 6, 1);
        for i in 0..8u64 {
            bus.ram.write_u8(down + i, 0xEE);
        }
        attach(&mut bus, cb);

        assert_eq!(bus.write_rtt_down(0, b"abcd"), 2);
        assert_eq!(bus.ram.read_u8(down + 6), Some(b'a'));
        assert_eq!(bus.ram.read_u8(down + 7), Some(b'b'));
        assert_eq!(bus.ram.read_u8(down), Some(0xEE));
        let desc = down_desc(cb, 3);
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_WR), Some(0));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_RD), Some(1));

        // RdOff == WrOff + 1: the only gap is the reserved byte, so the ring
        // is full and a further write must not move either cursor.
        write_u32_at(&mut bus, desc + CHAN_OFF_WR, 0);
        write_u32_at(&mut bus, desc + CHAN_OFF_RD, 1);
        bus.ram.write_u8(down, 0xEE);
        assert_eq!(bus.write_rtt_down(0, b"Z"), 0);
        assert_eq!(bus.ram.read_u8(down), Some(0xEE));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_WR), Some(0));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_RD), Some(1));

        // WrOff immediately behind RdOff (the other one-byte gap): also full.
        write_u32_at(&mut bus, desc + CHAN_OFF_WR, 7);
        write_u32_at(&mut bus, desc + CHAN_OFF_RD, 0);
        bus.ram.write_u8(down + 7, 0xEE);
        assert_eq!(bus.write_rtt_down(0, b"Z"), 0);
        assert_eq!(bus.ram.read_u8(down + 7), Some(0xEE));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_WR), Some(7));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_RD), Some(0));

        write_u32_at(&mut bus, desc + CHAN_OFF_WR, 0);
        write_u32_at(&mut bus, desc + CHAN_OFF_RD, 0);
        write_u32_at(&mut bus, desc + CHAN_OFF_FLAGS, 1 << 24);
        bus.ram.write_u8(down, 0xEE);
        assert_eq!(bus.write_rtt_down(0, b"Q"), 0);
        assert_eq!(bus.ram.read_u8(down), Some(0xEE));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_WR), Some(0));
        assert_eq!(bus.ram.read_u32(desc + CHAN_OFF_RD), Some(0));
    }

    #[test]
    fn write_down_uses_each_index_and_refuses_an_alias_onto_up() {
        let mut bus = SystemBus::new();
        let cb = 0x2000_0000u64;
        let up = 0x2000_1000u64;
        let down0 = 0x2000_2000u64;
        let down1 = 0x2000_3000u64;
        setup_cb(&mut bus, cb, up, 16, 0, 0);
        setup_down(&mut bus, cb, 2, down0, 8, 0, 0);
        let desc1 = down_desc(cb, 2) + CHAN_SIZE;
        write_u32_at(&mut bus, cb + CB_OFF_MAX_DOWN, 2);
        write_u32_at(&mut bus, desc1 + CHAN_OFF_PBUFFER, down1 as u32);
        write_u32_at(&mut bus, desc1 + CHAN_OFF_SIZE, 8);
        write_u32_at(&mut bus, desc1 + CHAN_OFF_WR, 0);
        write_u32_at(&mut bus, desc1 + CHAN_OFF_RD, 0);
        attach(&mut bus, cb);

        assert_eq!(bus.write_rtt_down(1, b"Z"), 1);
        assert_eq!(bus.ram.read_u8(down1), Some(b'Z'));
        assert_eq!(bus.ram.read_u32(desc1 + CHAN_OFF_WR), Some(1));
        assert_eq!(bus.ram.read_u32(desc1 + CHAN_OFF_RD), Some(0));
        assert_ne!(bus.ram.read_u8(down0), Some(b'Z'));

        // MaxNumUpBuffers == 0 must not treat aUp[0] as aDown[0].
        write_u32_at(&mut bus, cb + CB_OFF_MAX_UP, 0);
        bus.ram.write_u8(up, 0xEE);
        assert_eq!(bus.write_rtt_down(0, b"Q"), 0);
        assert_eq!(bus.ram.read_u8(up), Some(0xEE));

        // No ID: do not compute aDown even when the counts look real.
        write_u32_at(&mut bus, cb + CB_OFF_MAX_UP, 2);
        for i in 0..16u64 {
            bus.ram.write_u8(cb + i, 0);
        }
        bus.ram.write_u8(down0, 0xEE);
        assert_eq!(bus.write_rtt_down(0, b"Q"), 0);
        assert_eq!(bus.ram.read_u8(down0), Some(0xEE));
    }
}
