// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C5/C6 RMT transmitter model.
//!
//! The model covers the C5/C6 two-channel TX path, shared symbol RAM, the TX
//! configuration/status/interrupt registers, and timed GPIO-matrix output.
//! It does not model RX sampling, carrier modulation, DMA, or the fractional
//! RMT clock divider. The C6 register layout is from Espressif's `esp32c6.svd`
//! v9; C5 uses the matching TX/RAM layout in its TRM and `rmt_struct.h`. GPIO
//! output signal indices differ: 71/72 on C6 and 64/65 on C5.

use crate::bus::SystemBus;
use crate::peripherals::esp32c3::gpio::Esp32c3Gpio;
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};

const TX_CHANNELS: usize = 2;
const CHANNELS: usize = 4;
const BLOCK_WORDS: usize = 48;
const MEM_WORDS: usize = CHANNELS * BLOCK_WORDS;
const MEM_START: u64 = 0x400;
const MEM_END: u64 = MEM_START + (MEM_WORDS as u64) * 4;
const TX_START: u32 = 1;
const TX_WT_MASK: u32 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 23) | (1 << 24);
const RX_CONF1_WT_MASK: u32 = (1 << 1) | (1 << 2) | (1 << 14) | (1 << 15);
const ENTRY_DURATION_MASK: u32 = 0x7FFF;
const RMT_SOURCE_C6: u32 = 49;
const RMT_SOURCE_HZ: u64 = 80_000_000;

#[derive(Debug, Clone, Copy)]
enum GpioProfile {
    C5,
    C6,
}

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

#[derive(Debug)]
pub struct Esp32c6Rmt {
    source_id: u32,
    gpio_profile: GpioProfile,
    output_signal_base: u32,
    cpu_cycles_per_source_tick: u64,
    tx_conf0: [u32; TX_CHANNELS],
    rx_conf0: [u32; 2],
    rx_conf1: [u32; 2],
    regs: [u32; 0x34],
    mem: [u32; MEM_WORDS],
    fifo_write: [usize; CHANNELS],
    int_raw: u32,
    int_ena: u32,
    playback: Option<Playback>,
    tx_start_count: u32,
    clock: Option<CycleClock>,
    last_bus_tick_cycle: Option<u64>,
}

impl Esp32c6Rmt {
    pub fn new(source_id: u32) -> Self {
        Self::new_with_profile(source_id, GpioProfile::C6, 71, 160_000_000)
    }

    /// Construct the same TX/RAM IP for ESP32-C5's distinct GPIO model and
    /// signal numbering. C5 board profiles use the fixed 80 MHz PLL source.
    pub fn new_for_c5(source_id: u32, cpu_clock_hz: u32) -> Self {
        Self::new_with_profile(source_id, GpioProfile::C5, 64, cpu_clock_hz)
    }

    fn new_with_profile(
        source_id: u32,
        gpio_profile: GpioProfile,
        output_signal_base: u32,
        cpu_clock_hz: u32,
    ) -> Self {
        let mut regs = [0; 0x34];
        // SVD-stated reset for SYS_CONF (APB FIFO mode, memory clock on,
        // divider 1, 80 MHz source active). Other inherited reset values are
        // left at zero rather than promoted to claims.
        regs[0x68 / 4] = 0x0500_0010;
        Self {
            source_id,
            gpio_profile,
            output_signal_base,
            cpu_cycles_per_source_tick: (u64::from(cpu_clock_hz) / RMT_SOURCE_HZ).max(1),
            tx_conf0: [0x0071_0200; TX_CHANNELS],
            rx_conf0: [0x30FF_FF02; 2],
            rx_conf1: [0x0000_01E8; 2],
            regs,
            mem: [0; MEM_WORDS],
            fifo_write: [0; CHANNELS],
            int_raw: 0,
            int_ena: 0,
            playback: None,
            tx_start_count: 0,
            clock: None,
            last_bus_tick_cycle: None,
        }
    }

    pub fn tx_start_count(&self) -> u32 {
        self.tx_start_count
    }

    fn tx_conf_index(offset: u64) -> Option<usize> {
        match offset {
            0x10 => Some(0),
            0x14 => Some(1),
            _ => None,
        }
    }

    fn rx_conf0_index(offset: u64) -> Option<usize> {
        match offset {
            0x18 => Some(0),
            0x20 => Some(1),
            _ => None,
        }
    }

    fn rx_conf1_index(offset: u64) -> Option<usize> {
        match offset {
            0x1C => Some(0),
            0x24 => Some(1),
            _ => None,
        }
    }

    fn tx_mem_words(&self, channel: usize) -> usize {
        let raw = if channel < TX_CHANNELS {
            (self.tx_conf0[channel] >> 16) & 0x7
        } else {
            (self.rx_conf0[channel - TX_CHANNELS] >> 23) & 0x7
        } as usize;
        (raw.max(1) * BLOCK_WORDS).min(MEM_WORDS - channel * BLOCK_WORDS)
    }

    fn decode_edges(&self, channel: usize) -> (Vec<(u64, bool)>, u64) {
        let base = channel * BLOCK_WORDS;
        let len = self.tx_mem_words(channel);
        // The C5/C6 board profiles use an 80 MHz RMT source. DIV_CNT is
        // integer-only here; fractional divider and source changes are outside
        // this model.
        let cycles_per_tick = (((self.tx_conf0[channel] >> 8) & 0xFF).max(1) as u64)
            * self.cpu_cycles_per_source_tick;
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
        (edges, cycle)
    }

    fn start_tx(&mut self, channel: usize) {
        self.tx_start_count = self.tx_start_count.saturating_add(1);
        let started_cycle = self.clock.as_ref().map_or(0, |clock| clock.now());
        self.last_bus_tick_cycle = self.clock.as_ref().map(|clock| clock.now());
        let (edges, duration_cycles) = self.decode_edges(channel);
        if !edges.is_empty() {
            self.playback = Some(Playback {
                channel,
                signal: self.output_signal_base + channel as u32,
                edges,
                duration_cycles,
                next: 0,
                elapsed: 0,
                started_cycle,
                pads: Vec::new(),
                resolved: false,
            });
        } else {
            self.int_raw |= 1 << channel;
        }
    }

    fn mem_index(offset: u64) -> Option<usize> {
        if (MEM_START..MEM_END).contains(&offset) {
            Some(((offset - MEM_START) / 4) as usize)
        } else {
            None
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        if let Some(ch) = Self::tx_conf_index(offset) {
            return self.tx_conf0[ch];
        }
        if let Some(ch) = Self::rx_conf0_index(offset) {
            return self.rx_conf0[ch];
        }
        if let Some(ch) = Self::rx_conf1_index(offset) {
            return self.rx_conf1[ch];
        }
        match offset {
            0x00 | 0x04 | 0x08 | 0x0C => {
                let ch = (offset / 4) as usize;
                let count = self.fifo_write[ch];
                if count == 0 {
                    0
                } else {
                    let len = self.tx_mem_words(ch);
                    self.mem[ch * BLOCK_WORDS + (count - 1) % len]
                }
            }
            0x28 | 0x2C | 0x30 | 0x34 => 0,
            0x38 => self.int_raw,
            0x3C => self.int_raw & self.int_ena,
            0x40 => self.int_ena,
            0x44 => 0,
            _ => Self::mem_index(offset)
                .and_then(|i| self.mem.get(i).copied())
                .unwrap_or_else(|| self.regs.get((offset / 4) as usize).copied().unwrap_or(0)),
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if let Some(ch) = Self::tx_conf_index(offset) {
            self.tx_conf0[ch] = value & !TX_WT_MASK;
            if value & TX_START != 0 {
                self.start_tx(ch);
            }
            return;
        }
        if let Some(ch) = Self::rx_conf0_index(offset) {
            self.rx_conf0[ch] = value;
            return;
        }
        if let Some(ch) = Self::rx_conf1_index(offset) {
            self.rx_conf1[ch] = value & !RX_CONF1_WT_MASK;
            return;
        }
        if (0x00..=0x0C).contains(&offset) && offset % 4 == 0 {
            let ch = (offset / 4) as usize;
            let len = self.tx_mem_words(ch);
            let slot = ch * BLOCK_WORDS + self.fifo_write[ch] % len;
            self.mem[slot] = value;
            self.fifo_write[ch] = self.fifo_write[ch].wrapping_add(1);
            return;
        }
        if let Some(index) = Self::mem_index(offset) {
            self.mem[index] = value;
            return;
        }
        match offset {
            0x28..=0x34 => {} // channel status is read-only
            0x38 => self.int_raw &= !value,
            0x3C => {} // INT_ST is read-only
            0x40 => self.int_ena = value & 0x3FFF,
            0x44 => self.int_raw &= !value,
            0x70 => {} // REF_CNT_RST write triggers are not retained
            _ => {
                if let Some(reg) = self.regs.get_mut((offset / 4) as usize) {
                    *reg = value;
                }
            }
        }
    }

    fn with_c6_gpio<R>(bus: &mut dyn Bus, f: impl FnOnce(&mut Esp32c3Gpio) -> R) -> Option<R> {
        let system = bus.as_any_mut()?.downcast_mut::<SystemBus>()?;
        let index = system.find_peripheral_index_by_name("gpio")?;
        let gpio = system.peripherals[index]
            .dev
            .as_any_mut()?
            .downcast_mut::<Esp32c3Gpio>()?;
        Some(f(gpio))
    }

    fn with_c5_gpio<R>(
        bus: &mut dyn Bus,
        f: impl FnOnce(&mut crate::peripherals::esp32c5::gpio::Esp32c5Gpio) -> R,
    ) -> Option<R> {
        let system = bus.as_any_mut()?.downcast_mut::<SystemBus>()?;
        let index = system.find_peripheral_index_by_name("gpio")?;
        let gpio = system.peripherals[index]
            .dev
            .as_any_mut()?
            .downcast_mut::<crate::peripherals::esp32c5::gpio::Esp32c5Gpio>()?;
        Some(f(gpio))
    }

    fn output_pads(&self, bus: &mut dyn Bus, signal: u32) -> Vec<u8> {
        match self.gpio_profile {
            GpioProfile::C5 => Self::with_c5_gpio(bus, |gpio| gpio.pads_for_output_signal(signal)),
            GpioProfile::C6 => Self::with_c6_gpio(bus, |gpio| gpio.pads_for_output_signal(signal)),
        }
        .unwrap_or_default()
    }

    fn drive_output_at(&self, bus: &mut dyn Bus, pads: &[u8], level: bool, cycle: u64) {
        match self.gpio_profile {
            GpioProfile::C5 => {
                Self::with_c5_gpio(bus, |gpio| {
                    for &pad in pads {
                        gpio.drive_pad_output_at(pad, level, cycle);
                    }
                });
            }
            GpioProfile::C6 => {
                Self::with_c6_gpio(bus, |gpio| {
                    for &pad in pads {
                        gpio.drive_pad_output_at(pad, level, cycle);
                    }
                });
            }
        }
    }
}

impl Default for Esp32c6Rmt {
    fn default() -> Self {
        Self::new(RMT_SOURCE_C6)
    }
}

impl Peripheral for Esp32c6Rmt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let word = (self.read_word(aligned) & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_word(aligned, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let explicit_irqs = (self.int_raw & self.int_ena != 0).then(|| vec![self.source_id]);
        PeripheralTickResult {
            explicit_irqs,
            ..PeripheralTickResult::default()
        }
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
        if !edges.is_empty() {
            for (offset, level) in edges {
                self.drive_output_at(bus, &pads, level, started_cycle.saturating_add(offset));
            }
        }
        if done {
            self.int_raw |= 1 << channel;
            self.playback = None;
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_registers_and_direct_symbol_ram_are_backed() {
        let mut rmt = Esp32c6Rmt::default();
        rmt.write(0x400, 0x34).unwrap();
        assert_eq!(rmt.read_word(0x400), 0x34);
        rmt.write(0x401, 0x12).unwrap();
        assert_eq!(rmt.read_word(0x400), 0x1234);
        rmt.write(0x10, 0x01).unwrap();
        assert_eq!(rmt.read_word(0x38) & 1, 1);
        assert_eq!(rmt.read_word(0x10) & TX_START, 0);
        assert_eq!(rmt.tx_start_count(), 1);
    }

    #[test]
    fn status_interrupt_is_masked_and_write_one_cleared() {
        let mut rmt = Esp32c6Rmt::default();
        rmt.write(0x10, 1).unwrap();
        assert_eq!(rmt.read_word(0x3C), 0);
        rmt.write(0x40, 1).unwrap();
        assert_eq!(rmt.read_word(0x3C), 1);
        rmt.write(0x44, 1).unwrap();
        assert_eq!(rmt.read_word(0x38), 0);
    }
}
