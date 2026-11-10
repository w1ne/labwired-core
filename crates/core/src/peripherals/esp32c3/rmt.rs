// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 RMT transmitter model.
//!
//! The C3 has two TX channels, two RX channels, and 48 words of symbol RAM per
//! channel. This model executes the timed TX symbol stream at the configured
//! integer divider from the fixed 80 MHz APB clock, routes TX0/TX1 through the
//! C3 GPIO matrix (signals 51/52), and raises TX_END when the stream completes.
//! RX sampling, carrier modulation, DMA, and fractional clock division are
//! outside this model.

use crate::bus::SystemBus;
use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// ETS_RMT_INTR_SOURCE on ESP32-C3 (`soc/interrupts.h` / rmt.yaml).
pub const RMT_SOURCE_C3: u32 = 28;

const TX_CHANNELS: usize = 2;
const CHANNELS: usize = 4;
const WORDS_PER_CHANNEL: usize = 48;
const MEMORY_WORDS: usize = CHANNELS * WORDS_PER_CHANNEL;
const MEMORY_START: u64 = 0x400;
const MEMORY_END: u64 = MEMORY_START + MEMORY_WORDS as u64 * 4;
const APB_CLOCK_HZ: u64 = 80_000_000;
const TX_START_BIT: u32 = 1 << 0;
const TX_CONF0_WT_MASK: u32 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 23) | (1 << 24);
const ENTRY_DURATION_MASK: u32 = 0x7FFF;

/// C3 GPIO matrix output signal indices (`gpio_sig_map.h`).
const RMT_OUT0_SIGNAL: u32 = 51;

#[derive(Debug)]
struct Playback {
    channel: usize,
    signal: u32,
    edges: Vec<(u64, bool)>,
    duration_cycles: u64,
    next: usize,
    elapsed: u64,
    started_cycle: u64,
    pads: Vec<u8>,
    resolved: bool,
}

/// Logic-tap state: pin N = TX channel N.
struct RmtTap {
    tap: crate::logic_capture::LogicTap,
    watched: Vec<(u8, u32)>,
}

impl std::fmt::Debug for RmtTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RmtTap")
            .field("watched", &self.watched)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct Esp32c3Rmt {
    source_id: u32,
    cpu_cycles_per_source_tick: u64,
    /// CH0/CH1 TX CONF0 @ 0x10 / 0x14.
    tx_conf0: [u32; TX_CHANNELS],
    /// RX configuration/status and sparse control-register storage.
    other: [u32; 0x40],
    int_raw: u32,
    int_ena: u32,
    mem: [u32; MEMORY_WORDS],
    fifo_write: [usize; CHANNELS],
    tx_start_count: u32,
    playback: Option<Playback>,
    clock: Option<CycleClock>,
    last_bus_tick_cycle: Option<u64>,
    tap: Option<RmtTap>,
}

impl Esp32c3Rmt {
    pub fn new(source_id: u32, cpu_clock_hz: u32) -> Self {
        Self {
            source_id,
            cpu_cycles_per_source_tick: (u64::from(cpu_clock_hz) / APB_CLOCK_HZ).max(1),
            tx_conf0: [0; TX_CHANNELS],
            other: [0; 0x40],
            int_raw: 0,
            int_ena: 0,
            mem: [0; MEMORY_WORDS],
            fifo_write: [0; CHANNELS],
            tx_start_count: 0,
            playback: None,
            clock: None,
            last_bus_tick_cycle: None,
            tap: None,
        }
    }

    pub fn new_default() -> Self {
        Self::new(RMT_SOURCE_C3, 160_000_000)
    }

    pub fn tx_start_count(&self) -> u32 {
        self.tx_start_count
    }

    fn memory_index(offset: u64) -> Option<usize> {
        if (MEMORY_START..MEMORY_END).contains(&offset) {
            Some(((offset - MEMORY_START) / 4) as usize)
        } else {
            None
        }
    }

    fn tx_conf_index(offset: u64) -> Option<usize> {
        match offset {
            0x10 => Some(0),
            0x14 => Some(1),
            _ => None,
        }
    }

    fn tx_mem_words(&self, channel: usize) -> usize {
        let configured_blocks = ((self.tx_conf0[channel] >> 16) & 0x7) as usize;
        (configured_blocks.max(1) * WORDS_PER_CHANNEL)
            .min(MEMORY_WORDS.saturating_sub(channel * WORDS_PER_CHANNEL))
    }

    /// Decode RMT symbol words into transitions measured in CPU cycles.
    fn decode_edges(&self, channel: usize) -> (Vec<(u64, bool)>, u64) {
        let base = channel * WORDS_PER_CHANNEL;
        let len = self.tx_mem_words(channel);
        let divider = (((self.tx_conf0[channel] >> 8) & 0xFF).max(1)) as u64;
        let cycles_per_tick = divider.saturating_mul(self.cpu_cycles_per_source_tick);
        let mut edges = Vec::new();
        let mut level = false;
        let mut cycle = 0u64;

        for word in &self.mem[base..base + len] {
            for (duration, next_level) in [
                ((word & ENTRY_DURATION_MASK) as u64, word & (1 << 15) != 0),
                (
                    ((word >> 16) & ENTRY_DURATION_MASK) as u64,
                    word & (1 << 31) != 0,
                ),
            ] {
                if duration == 0 {
                    return (edges, cycle);
                }
                if level != next_level {
                    edges.push((cycle, next_level));
                    level = next_level;
                }
                cycle = cycle.saturating_add(duration.saturating_mul(cycles_per_tick));
            }
        }

        // The C3 transmitter holds the configured idle level after the final
        // symbol when IDLE_OUT_EN is set. Preserve that final edge as part of
        // the observable stream.
        let conf = self.tx_conf0[channel];
        if conf & (1 << 6) != 0 {
            let idle_level = conf & (1 << 5) != 0;
            if level != idle_level {
                edges.push((cycle, idle_level));
            }
        }
        (edges, cycle)
    }

    fn start_tx(&mut self, channel: usize) {
        self.tx_start_count = self.tx_start_count.saturating_add(1);
        let started_cycle = self.clock.as_ref().map_or(0, CycleClock::now);
        self.last_bus_tick_cycle = self.clock.as_ref().map(CycleClock::now);
        let (edges, duration_cycles) = self.decode_edges(channel);
        if edges.is_empty() || duration_cycles == 0 {
            self.int_raw |= 1 << channel;
            return;
        }
        self.playback = Some(Playback {
            channel,
            signal: RMT_OUT0_SIGNAL + channel as u32,
            edges,
            duration_cycles,
            next: 0,
            elapsed: 0,
            started_cycle,
            pads: Vec::new(),
            resolved: false,
        });
    }

    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    fn with_c3_gpio<R>(bus: &mut dyn Bus, f: impl FnOnce(&mut Esp32c3Gpio) -> R) -> Option<R> {
        let system = bus.as_any_mut()?.downcast_mut::<SystemBus>()?;
        let index = system.find_peripheral_index_by_name("gpio")?;
        let gpio = system.peripherals[index]
            .dev
            .as_any_mut()?
            .downcast_mut::<Esp32c3Gpio>()?;
        Some(f(gpio))
    }

    fn output_pads(&self, bus: &mut dyn Bus, signal: u32) -> Vec<u8> {
        Self::with_c3_gpio(bus, |gpio| gpio.enabled_pads_for_output_signal(signal))
            .unwrap_or_default()
    }

    fn drive_output_at(&self, bus: &mut dyn Bus, pads: &[u8], level: bool, cycle: u64) {
        Self::with_c3_gpio(bus, |gpio| {
            for &pad in pads {
                gpio.drive_pad_output_at(pad, level, cycle);
            }
        });
    }

    fn tap_edge(&self, channel: usize, level: bool) {
        let Some(tap) = &self.tap else {
            return;
        };
        for &(pin, line) in &tap.watched {
            if pin == channel as u8 {
                tap.tap.push(line, level);
                tap.tap.bump_clock();
            }
        }
    }

    fn write_conf0(&mut self, channel: usize, value: u32) {
        if channel >= TX_CHANNELS {
            return;
        }
        self.tx_conf0[channel] = value & !TX_CONF0_WT_MASK;
        if value & TX_START_BIT != 0 {
            self.int_raw &= !(1 << channel);
            self.start_tx(channel);
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        if let Some(channel) = Self::tx_conf_index(offset) {
            return self.tx_conf0[channel];
        }
        if let Some(index) = Self::memory_index(offset) {
            return self.mem[index];
        }
        match offset {
            0x00..=0x0C if offset % 4 == 0 => {
                let channel = (offset / 4) as usize;
                let count = self.fifo_write[channel];
                if count == 0 {
                    0
                } else {
                    let len = self.tx_mem_words(channel);
                    self.mem[channel * WORDS_PER_CHANNEL + (count - 1) % len]
                }
            }
            0x28..=0x34 if offset % 4 == 0 => self.other[(offset - 0x28) as usize / 4],
            0x38 => self.int_raw,
            0x3C => self.int_st(),
            0x40 => self.int_ena,
            0x44 => 0,
            off if off < 0x100 && off % 4 == 0 => self.other[(off / 4) as usize],
            _off => {
                crate::census_reg!("esp32c3.rmt:Esp32c3Rmt", _off, "read");
                0
            }
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if let Some(channel) = Self::tx_conf_index(offset) {
            self.write_conf0(channel, value);
            return;
        }
        if let Some(index) = Self::memory_index(offset) {
            self.mem[index] = value;
            return;
        }
        match offset {
            0x00..=0x0C if offset % 4 == 0 => {
                let channel = (offset / 4) as usize;
                let len = self.tx_mem_words(channel);
                let index = channel * WORDS_PER_CHANNEL + self.fifo_write[channel] % len;
                self.mem[index] = value;
                self.fifo_write[channel] = self.fifo_write[channel].wrapping_add(1);
            }
            0x28..=0x34 if offset % 4 == 0 => {} // channel status is read-only
            0x38 => {}                           // INT_RAW is set by hardware; INT_CLR clears it
            0x3C => {}                           // INT_ST is read-only
            0x40 => self.int_ena = value,
            0x44 => self.int_raw &= !value,
            0x10..=0x24 if offset % 4 == 0 => {
                self.other[offset as usize / 4] = value;
            }
            off if off < 0x100 && off % 4 == 0 => self.other[(off / 4) as usize] = value,
            _off => crate::census_reg!("esp32c3.rmt:Esp32c3Rmt", _off, "write"),
        }
    }
}

impl Default for Esp32c3Rmt {
    fn default() -> Self {
        Self::new_default()
    }
}

impl Peripheral for Esp32c3Rmt {
    fn uses_scheduler(&self) -> bool {
        true
    }

    fn needs_bus_tick(&self) -> bool {
        self.playback.is_some()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        if self.playback.is_none() {
            return;
        }
        let now = self.clock.as_ref().map(CycleClock::now);
        let elapsed = match (self.last_bus_tick_cycle, now) {
            (Some(last), Some(now)) if now > last => now - last,
            _ => 1,
        };
        if let Some(now) = now {
            self.last_bus_tick_cycle = Some(now);
        }

        if !self.playback.as_ref().unwrap().resolved {
            let signal = self.playback.as_ref().unwrap().signal;
            let pads = self.output_pads(bus, signal);
            let playback = self.playback.as_mut().unwrap();
            playback.resolved = true;
            playback.pads = pads;
        }

        let (edges, started_cycle, pads, channel, done) = {
            let playback = self.playback.as_mut().unwrap();
            let through = playback.elapsed.saturating_add(elapsed.saturating_sub(1));
            let mut edges = Vec::new();
            while playback.next < playback.edges.len() && playback.edges[playback.next].0 <= through
            {
                edges.push(playback.edges[playback.next]);
                playback.next += 1;
            }
            playback.elapsed = playback.elapsed.saturating_add(elapsed);
            (
                edges,
                playback.started_cycle,
                playback.pads.clone(),
                playback.channel,
                playback.next >= playback.edges.len()
                    && playback.elapsed >= playback.duration_cycles,
            )
        };
        for (offset, level) in edges {
            self.drive_output_at(bus, &pads, level, started_cycle.saturating_add(offset));
            self.tap_edge(channel, level);
        }
        if done {
            self.int_raw |= 1 << channel;
            self.playback = None;
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let word = (self.read_word(aligned) & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(aligned, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let explicit_irqs = (self.int_st() != 0).then(|| vec![self.source_id]);
        PeripheralTickResult {
            explicit_irqs,
            ..PeripheralTickResult::default()
        }
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.int_st() != 0 {
            out.push(self.source_id);
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn inspect(
        &self,
        base: u64,
        name: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> crate::inspect::PeripheralInspect {
        let mut view = crate::inspect::default_inspect(self, base, name, opts);
        view.kind = "rmt".into();
        view.artifacts.push(crate::inspect::Artifact {
            kind: "rmt_tx".into(),
            id: "tx_start".into(),
            meta: serde_json::json!({ "count": self.tx_start_count }),
            bytes: None,
        });
        view
    }

    fn install_logic_tap(
        &mut self,
        tap: &crate::logic_capture::LogicTap,
        watched: &[(u8, u32)],
    ) -> bool {
        if watched.is_empty() {
            self.tap = None;
        } else {
            self.tap = Some(RmtTap {
                tap: tap.clone(),
                watched: watched.to_vec(),
            });
        }
        true
    }
}
