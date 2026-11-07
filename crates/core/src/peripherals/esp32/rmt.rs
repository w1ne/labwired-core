// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic RMT transmit model.
//!
//! This model implements the classic ESP32 RMT control/status register offsets,
//! interrupt registers, and 2 KiB RMTMEM aperture. Channels 0–7 can transmit
//! independent symbol streams onto their GPIO-matrix outputs with configured
//! divider timing; each channel latches TX_END after its stream elapses. RX
//! retains register storage but does not run a waveform engine.

use crate::bus::SystemBus;
use crate::peripherals::esp32::gpio::{Esp32Gpio, RMT_SIG_OUT0};
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};

const CHANNELS: usize = 8;
const MEM_WORDS: usize = 512;
const MEM_BASE: u64 = 0x800;
const MEM_END: u64 = 0x1000;
const CPU_HZ: u64 = 240_000_000;
const APB_HZ: u64 = 80_000_000;

#[derive(Debug)]
struct TxPlayback {
    edges: Vec<(u64, bool)>,
    next_edge: usize,
    elapsed_cycles: u64,
    duration_cycles: u64,
    started_at: u64,
    pads: Vec<u8>,
    pads_resolved: bool,
}

#[derive(Debug)]
pub struct Esp32Rmt {
    source_id: u32,
    conf0: [u32; CHANNELS],
    conf1: [u32; CHANNELS],
    status: [u32; CHANNELS],
    addr: [u32; CHANNELS],
    mem: [u32; MEM_WORDS],
    int_raw: u32,
    int_ena: u32,
    tx_lim: [u32; CHANNELS],
    carrier_duty: [u32; CHANNELS],
    apb_conf: u32,
    date: u32,
    playback: [Option<TxPlayback>; CHANNELS],
    clock: Option<CycleClock>,
    last_bus_tick_cycle: Option<u64>,
}

impl Esp32Rmt {
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            conf0: [0x3110_0002; CHANNELS],
            conf1: [0x0000_0F20; CHANNELS],
            status: [0; CHANNELS],
            addr: [0; CHANNELS],
            mem: [0; MEM_WORDS],
            int_raw: 0,
            int_ena: 0,
            tx_lim: [128; CHANNELS],
            carrier_duty: [0x0040_0040; CHANNELS],
            apb_conf: 0,
            date: 0x1602_2600,
            playback: std::array::from_fn(|_| None),
            clock: None,
            last_bus_tick_cycle: None,
        }
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn channel_conf0(offset: u64) -> Option<usize> {
        (0..CHANNELS).find(|ch| offset == 0x20 + (*ch as u64) * 8)
    }

    fn channel_conf1(offset: u64) -> Option<usize> {
        (0..CHANNELS).find(|ch| offset == 0x24 + (*ch as u64) * 8)
    }

    fn channel_status(offset: u64) -> Option<usize> {
        (0..CHANNELS).find(|ch| offset == 0x60 + (*ch as u64) * 4)
    }

    fn channel_addr(offset: u64) -> Option<usize> {
        (0..CHANNELS).find(|ch| offset == 0x80 + (*ch as u64) * 4)
    }

    fn mem_index(offset: u64) -> Option<usize> {
        if (MEM_BASE..MEM_END).contains(&offset) {
            Some(((offset - MEM_BASE) / 4) as usize)
        } else {
            None
        }
    }

    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    fn tx_tick_cycles(&self, channel: usize) -> u64 {
        // The board smoke selects REF_ALWAYS_ON (APB clock). The chip's APB
        // clock is 80 MHz while the LX6 runs at 240 MHz.
        let divider = (self.conf0[channel] & 0xff).max(1) as u64;
        (CPU_HZ / APB_HZ).saturating_mul(divider)
    }

    fn arm_tx(&mut self, channel: usize) {
        if self.playback[channel].is_some() {
            return;
        }
        // Classic ESP32 RMT has 64 words per channel in RMTMEM. Stop at the
        // first zero-duration half-symbol; this is the hardware end marker.
        let per_tick = self.tx_tick_cycles(channel);
        let mut edges = Vec::new();
        let mut offset = 0u64;
        let mut level = false;
        let mut terminated = false;
        let mem_start = channel * 64;
        for word in self.mem[mem_start..mem_start + 64].iter().copied() {
            for (duration, next_level) in [
                ((word & 0x7fff) as u64, word & (1 << 15) != 0),
                (((word >> 16) & 0x7fff) as u64, word & (1 << 31) != 0),
            ] {
                if duration == 0 {
                    terminated = true;
                    break;
                }
                if next_level != level {
                    edges.push((offset, next_level));
                    level = next_level;
                }
                offset = offset.saturating_add(duration.saturating_mul(per_tick));
            }
            if terminated {
                break;
            }
        }
        if terminated && offset != 0 {
            self.playback[channel] = Some(TxPlayback {
                edges,
                next_edge: 0,
                elapsed_cycles: 0,
                duration_cycles: offset,
                started_at: self.now(),
                pads: Vec::new(),
                pads_resolved: false,
            });
        } else {
            // A stream without an end marker does not complete this modeled
            // non-continuous TX operation.
            self.playback[channel] = None;
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        if let Some(i) = Self::channel_conf0(offset) {
            return self.conf0[i];
        }
        if let Some(i) = Self::channel_conf1(offset) {
            return self.conf1[i];
        }
        if let Some(i) = Self::channel_status(offset) {
            return self.status[i];
        }
        if let Some(i) = Self::channel_addr(offset) {
            return self.addr[i];
        }
        if let Some(i) = Self::mem_index(offset) {
            return self.mem[i];
        }
        match offset {
            0xA0 => self.int_raw,
            0xA4 => self.int_st(),
            0xA8 => self.int_ena,
            0xAC => 0,
            0xB0..=0xCC if (offset - 0xB0) % 4 == 0 => {
                self.carrier_duty[((offset - 0xB0) / 4) as usize]
            }
            0xD0..=0xEC if (offset - 0xD0) % 4 == 0 => self.tx_lim[((offset - 0xD0) / 4) as usize],
            0xF0 => self.apb_conf,
            0xFC => self.date,
            // CHnDATA is the APB FIFO port. The smoke uses the direct RMTMEM
            // aperture; this narrow model exposes each FIFO channel's first
            // word but does not model its advancing APB read/write pointer.
            0x00..=0x1C if offset % 4 == 0 => self.mem[((offset / 4) as usize) * 64],
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if let Some(i) = Self::channel_conf0(offset) {
            self.conf0[i] = value;
            return;
        }
        if let Some(i) = Self::channel_conf1(offset) {
            let start = value & 1 != 0;
            // Start and memory reset fields self-clear. The other writable
            // configuration bits retain their written values.
            self.conf1[i] = value & !((1 << 0) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 16));
            if start {
                self.int_raw &= !(1 << i);
                self.arm_tx(i);
            }
            return;
        }
        if let Some(i) = Self::channel_addr(offset) {
            self.addr[i] = value;
            return;
        }
        if let Some(i) = Self::mem_index(offset) {
            self.mem[i] = value;
            return;
        }
        match offset {
            0xA0 => {}
            0xA8 => self.int_ena = value,
            0xAC => self.int_raw &= !value,
            0xB0..=0xCC if (offset - 0xB0) % 4 == 0 => {
                self.carrier_duty[((offset - 0xB0) / 4) as usize] = value
            }
            0xD0..=0xEC if (offset - 0xD0) % 4 == 0 => {
                self.tx_lim[((offset - 0xD0) / 4) as usize] = value
            }
            0xF0 => self.apb_conf = value,
            0x00..=0x1C if offset % 4 == 0 => {
                let index = ((offset / 4) as usize) * 64;
                self.mem[index] = value;
            }
            _ => {}
        }
    }

    fn with_gpio<R>(bus: &mut dyn Bus, f: impl FnOnce(&mut Esp32Gpio) -> R) -> Option<R> {
        let sb = bus.as_any_mut()?.downcast_mut::<SystemBus>()?;
        let idx = sb.find_peripheral_index_by_name("gpio")?;
        let gpio = sb.peripherals[idx]
            .dev
            .as_any_mut()?
            .downcast_mut::<Esp32Gpio>()?;
        Some(f(gpio))
    }
}

impl Peripheral for Esp32Rmt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        let shift = ((offset & 3) * 8) as u32;
        Ok(((word >> shift) & 0xff) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_offset = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        let mask = 0xffu32 << shift;
        let word = (self.read_word(word_offset) & !mask) | ((value as u32) << shift);
        self.write_word(word_offset, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let explicit_irqs = (self.int_st() != 0).then(|| vec![self.source_id]);
        PeripheralTickResult {
            explicit_irqs,
            ..PeripheralTickResult::default()
        }
    }

    fn needs_bus_tick(&self) -> bool {
        self.playback.iter().any(Option::is_some)
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        let now = self.clock.as_ref().map(CycleClock::now);
        let elapsed = match (self.last_bus_tick_cycle, now) {
            (Some(last), Some(now)) if now > last => now - last,
            _ => 1,
        };
        if let Some(now) = now {
            self.last_bus_tick_cycle = Some(now);
        }
        let mut pin_events = Vec::new();
        for channel in 0..CHANNELS {
            let Some(playback) = self.playback[channel].as_mut() else {
                continue;
            };
            if !playback.pads_resolved {
                let signal = RMT_SIG_OUT0 + channel as u32;
                playback.pads = Self::with_gpio(bus, |gpio| gpio.pads_for_output_signal(signal))
                    .unwrap_or_default();
                playback.pads_resolved = true;
            }

            let last_cycle = playback.elapsed_cycles.saturating_add(elapsed - 1);
            while let Some(&(offset, level)) = playback.edges.get(playback.next_edge) {
                if offset > last_cycle {
                    break;
                }
                let cycle = playback.started_at.saturating_add(offset);
                for &pin in &playback.pads {
                    pin_events.push((pin, cycle, level));
                }
                playback.next_edge += 1;
            }
            playback.elapsed_cycles = playback.elapsed_cycles.saturating_add(elapsed);
            if playback.elapsed_cycles >= playback.duration_cycles {
                self.int_raw |= 1 << channel;
                self.status[channel] = 0;
                self.playback[channel] = None;
            }
        }
        if !pin_events.is_empty() {
            Self::with_gpio(bus, |gpio| {
                for (pin, cycle, level) in pin_events {
                    gpio.drive_pad_output_at(pin, level, cycle);
                }
            });
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
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
}
