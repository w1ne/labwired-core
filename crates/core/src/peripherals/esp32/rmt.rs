// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic and ESP32-S2 RMT transmit model.
//!
//! The two chips share one RMT register design: per-channel CONF0/CONF1 pairs,
//! then STATUS, ADDR, the INT block, CARRIER_DUTY, TX_LIM and APB_CONF, with
//! the symbol RAM (RMTMEM) at a fixed offset and 64 words per channel. The
//! classic part has 8 channels and RMTMEM at +0x800; the S2 has 4 and RMTMEM
//! at +0x400, so every offset after the DATA ports moves ([`RmtLayout`]).
//! Offsets: ESP-IDF `soc/{esp32,esp32s2}/register/soc/rmt_reg.h`.
//!
//! Channels transmit independent symbol streams onto their GPIO-matrix outputs
//! with configured divider timing; each channel latches TX_END after its
//! stream elapses. TX_END for channel n is INT_RAW bit 3n (`RMT_LL_EVENT_TX_DONE`
//! in both chips' `hal/rmt_ll.h`). RX retains register storage but does not
//! run a waveform engine; carrier, continuous and loop modes are not modelled.

use crate::peripherals::pad_lines::PadLines;
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

const CPU_HZ: u64 = 240_000_000;
const APB_HZ: u64 = 80_000_000;
/// Symbol RAM words each channel owns (`SOC_RMT_MEM_WORDS_PER_CHANNEL`).
const WORDS_PER_CHANNEL: usize = 64;
const MAX_CHANNELS: usize = 8;
/// Matrix output signal names, `RMT_SIG_OUTn_IDX` without the suffix.
const RMT_LINES: [&str; MAX_CHANNELS] = [
    "RMT_SIG_OUT0",
    "RMT_SIG_OUT1",
    "RMT_SIG_OUT2",
    "RMT_SIG_OUT3",
    "RMT_SIG_OUT4",
    "RMT_SIG_OUT5",
    "RMT_SIG_OUT6",
    "RMT_SIG_OUT7",
];

/// Where one chip puts the shared RMT register design.
#[derive(Debug, Clone, Copy)]
pub struct RmtLayout {
    channels: usize,
    conf0: u64,
    status: u64,
    addr: u64,
    int_raw: u64,
    carrier_duty: u64,
    tx_lim: u64,
    apb_conf: u64,
    mem_base: u64,
    conf0_reset: u32,
    /// CONF1 bits that hardware consumes and clears (TX_START and the
    /// write-only memory-pointer resets).
    conf1_self_clear: u32,
    /// GPIO-matrix output signal of channel 0 (`RMT_SIG_OUT0_IDX`).
    sig_out0: u32,
    date: u32,
}

impl RmtLayout {
    /// ESP32 (classic): 8 channels, RMTMEM at +0x800.
    pub const ESP32: Self = Self {
        channels: 8,
        conf0: 0x20,
        status: 0x60,
        addr: 0x80,
        int_raw: 0xA0,
        carrier_duty: 0xB0,
        tx_lim: 0xD0,
        apb_conf: 0xF0,
        mem_base: 0x800,
        conf0_reset: 0x3110_0002,
        conf1_self_clear: (1 << 0) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 16),
        sig_out0: crate::peripherals::esp32::gpio::RMT_SIG_OUT0,
        date: 0x1602_2600,
    };

    /// ESP32-S2: 4 channels, RMTMEM at +0x400 (`RMTMEM` in
    /// `esp32s2.peripherals.ld`), matrix signals 87..90 (`gpio_sig_map.h`).
    /// CONF0 resets with MEM_SIZE = 1 at [26:24] and the carrier bits at
    /// [29:27].
    pub const ESP32S2: Self = Self {
        channels: 4,
        conf0: 0x10,
        status: 0x30,
        addr: 0x40,
        int_raw: 0x50,
        carrier_duty: 0x60,
        tx_lim: 0x70,
        apb_conf: 0x80,
        mem_base: 0x400,
        conf0_reset: 0x3910_0002,
        conf1_self_clear: (1 << 0) | (1 << 2) | (1 << 3) | (1 << 4),
        sig_out0: 87,
        date: 0x1907_2601,
    };

    fn mem_end(&self) -> u64 {
        self.mem_base + (self.channels * WORDS_PER_CHANNEL * 4) as u64
    }

    fn per_channel(&self, offset: u64, base: u64, stride: u64) -> Option<usize> {
        let span = base..base + stride * self.channels as u64;
        (span.contains(&offset) && (offset - base) % stride == 0)
            .then(|| ((offset - base) / stride) as usize)
    }
}

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
    layout: RmtLayout,
    source_id: u32,
    conf0: [u32; MAX_CHANNELS],
    conf1: [u32; MAX_CHANNELS],
    status: [u32; MAX_CHANNELS],
    addr: [u32; MAX_CHANNELS],
    mem: Vec<u32>,
    int_raw: u32,
    int_ena: u32,
    tx_lim: [u32; MAX_CHANNELS],
    carrier_duty: [u32; MAX_CHANNELS],
    apb_conf: u32,
    playback: [Option<TxPlayback>; MAX_CHANNELS],
    /// TX_START writes that armed a stream (inspect artifact `rmt_tx`).
    tx_start_count: u64,
    /// Channel outputs as a wire, for a GPIO matrix that routes peripheral
    /// wires to pads (S2). The classic GPIO model declines it and the edges
    /// go through its output latch instead (`drive_matrix_pad`).
    lines: Arc<PadLines>,
    clock: Option<CycleClock>,
}

impl Esp32Rmt {
    pub fn new(source_id: u32) -> Self {
        Self::with_layout(RmtLayout::ESP32, source_id)
    }

    pub fn with_layout(layout: RmtLayout, source_id: u32) -> Self {
        Self {
            layout,
            source_id,
            conf0: [layout.conf0_reset; MAX_CHANNELS],
            conf1: [0x0000_0F20; MAX_CHANNELS],
            status: [0; MAX_CHANNELS],
            addr: [0; MAX_CHANNELS],
            mem: vec![0; layout.channels * WORDS_PER_CHANNEL],
            int_raw: 0,
            int_ena: 0,
            tx_lim: [128; MAX_CHANNELS],
            carrier_duty: [0x0040_0040; MAX_CHANNELS],
            apb_conf: 0,
            playback: std::array::from_fn(|_| None),
            tx_start_count: 0,
            lines: Arc::new(PadLines::new(
                &RMT_LINES[..layout.channels],
                &[false; MAX_CHANNELS][..layout.channels],
            )),
            clock: None,
        }
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    /// INT_RAW bit for channel `ch`'s TX_END (`RMT_LL_EVENT_TX_DONE`).
    fn tx_end_bit(channel: usize) -> u32 {
        1 << (channel * 3)
    }

    fn channel_conf0(&self, offset: u64) -> Option<usize> {
        self.layout.per_channel(offset, self.layout.conf0, 8)
    }

    fn channel_conf1(&self, offset: u64) -> Option<usize> {
        self.layout.per_channel(offset, self.layout.conf0 + 4, 8)
    }

    fn mem_index(&self, offset: u64) -> Option<usize> {
        (self.layout.mem_base..self.layout.mem_end())
            .contains(&offset)
            .then(|| ((offset - self.layout.mem_base) / 4) as usize)
    }

    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    fn tx_tick_cycles(&self, channel: usize) -> u64 {
        // REF_ALWAYS_ON selects the APB clock (what ESP-IDF picks on both
        // chips). APB is 80 MHz while the core runs at 240 MHz.
        let divider = (self.conf0[channel] & 0xff).max(1) as u64;
        (CPU_HZ / APB_HZ).saturating_mul(divider)
    }

    fn arm_tx(&mut self, channel: usize) {
        if self.playback[channel].is_some() {
            return;
        }
        // Stop at the first zero-duration half-symbol; this is the hardware
        // end marker.
        let per_tick = self.tx_tick_cycles(channel);
        let mut edges = Vec::new();
        let mut offset = 0u64;
        let mut level = false;
        let mut terminated = false;
        let mem_start = channel * WORDS_PER_CHANNEL;
        for word in self.mem[mem_start..mem_start + WORDS_PER_CHANNEL]
            .iter()
            .copied()
        {
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
            self.tx_start_count += 1;
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
        let l = &self.layout;
        if let Some(i) = self.channel_conf0(offset) {
            return self.conf0[i];
        }
        if let Some(i) = self.channel_conf1(offset) {
            return self.conf1[i];
        }
        if let Some(i) = l.per_channel(offset, l.status, 4) {
            return self.status[i];
        }
        if let Some(i) = l.per_channel(offset, l.addr, 4) {
            return self.addr[i];
        }
        if let Some(i) = self.mem_index(offset) {
            return self.mem[i];
        }
        if let Some(i) = l.per_channel(offset, l.carrier_duty, 4) {
            return self.carrier_duty[i];
        }
        if let Some(i) = l.per_channel(offset, l.tx_lim, 4) {
            return self.tx_lim[i];
        }
        // CHnDATA is the APB FIFO port. Firmware here uses the direct RMTMEM
        // aperture; this narrow model exposes each FIFO channel's first word
        // but does not model its advancing APB read/write pointer.
        if let Some(i) = l.per_channel(offset, 0, 4) {
            return self.mem[i * WORDS_PER_CHANNEL];
        }
        match offset {
            o if o == l.int_raw => self.int_raw,
            o if o == l.int_raw + 4 => self.int_st(),
            o if o == l.int_raw + 8 => self.int_ena,
            o if o == l.apb_conf => self.apb_conf,
            0xFC => l.date,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let l = self.layout;
        if let Some(i) = self.channel_conf0(offset) {
            self.conf0[i] = value;
            return;
        }
        if let Some(i) = self.channel_conf1(offset) {
            let start = value & 1 != 0;
            // Start and memory reset fields self-clear. The other writable
            // configuration bits retain their written values.
            self.conf1[i] = value & !l.conf1_self_clear;
            if start {
                self.int_raw &= !Self::tx_end_bit(i);
                self.arm_tx(i);
            }
            return;
        }
        if let Some(i) = l.per_channel(offset, l.addr, 4) {
            self.addr[i] = value;
            return;
        }
        if let Some(i) = self.mem_index(offset) {
            self.mem[i] = value;
            return;
        }
        if let Some(i) = l.per_channel(offset, l.carrier_duty, 4) {
            self.carrier_duty[i] = value;
            return;
        }
        if let Some(i) = l.per_channel(offset, l.tx_lim, 4) {
            self.tx_lim[i] = value;
            return;
        }
        if let Some(i) = l.per_channel(offset, 0, 4) {
            self.mem[i * WORDS_PER_CHANNEL] = value;
            return;
        }
        match offset {
            o if o == l.int_raw + 8 => self.int_ena = value,
            o if o == l.int_raw + 12 => self.int_raw &= !value,
            o if o == l.apb_conf => self.apb_conf = value,
            _ => {}
        }
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
        let mut pin_events = Vec::new();
        for channel in 0..self.layout.channels {
            let Some(playback) = self.playback[channel].as_mut() else {
                continue;
            };
            if !playback.pads_resolved {
                let signal = self.layout.sig_out0 + channel as u32;
                bus.with_peripheral_mut("gpio", &mut |gpio| {
                    playback.pads = gpio.pads_for_output_signal(signal);
                });
                playback.pads_resolved = true;
            }

            // Cycles of this stream played so far, counted from its own start:
            // the bus may tick in batches, and a channel armed long after the
            // previous stream ended must not inherit that gap. Without a cycle
            // clock (hand-built test buses) each call is one cycle.
            let played = match now {
                Some(now) => now.saturating_sub(playback.started_at) + 1,
                None => playback.elapsed_cycles + 1,
            };
            let last_cycle = played - 1;
            while let Some(&(offset, level)) = playback.edges.get(playback.next_edge) {
                if offset > last_cycle {
                    break;
                }
                let cycle = playback.started_at.saturating_add(offset);
                self.lines.set_line_at(channel, level, cycle);
                for &pin in &playback.pads {
                    pin_events.push((pin, cycle, level));
                }
                playback.next_edge += 1;
            }
            playback.elapsed_cycles = played;
            if playback.elapsed_cycles >= playback.duration_cycles {
                self.int_raw |= Self::tx_end_bit(channel);
                self.status[channel] = 0;
                self.playback[channel] = None;
            }
        }
        if !pin_events.is_empty() {
            bus.with_peripheral_mut("gpio", &mut |gpio| {
                for &(pin, cycle, level) in &pin_events {
                    gpio.drive_matrix_pad(pin, level, cycle);
                }
            });
        }
    }

    fn matrix_output_lines(&mut self) -> Option<(u32, Arc<PadLines>)> {
        Some((self.layout.sig_out0, self.lines.clone()))
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.int_st() != 0 {
            out.push(self.source_id);
        }
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

    fn write32(rmt: &mut Esp32Rmt, offset: u64, value: u32) {
        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
            rmt.write(offset + i as u64, byte).unwrap();
        }
    }

    fn read32(rmt: &Esp32Rmt, offset: u64) -> u32 {
        u32::from_le_bytes(std::array::from_fn(|i| {
            rmt.read(offset + i as u64).unwrap()
        }))
    }

    /// An S2 RMT on a published cycle clock: ch1 holds one high-then-low
    /// symbol of 10 + 10 ticks and an end marker. Divider 1 = 3 CPU cycles
    /// per tick, so the stream is 60 cycles.
    fn s2_with_stream() -> (Esp32Rmt, CycleClock) {
        let mut rmt = Esp32Rmt::with_layout(RmtLayout::ESP32S2, 50);
        let clock = CycleClock::default();
        rmt.attach_cycle_clock(clock.clone());
        write32(&mut rmt, 0x18, 1); // CH1CONF0: DIV_CNT = 1
        write32(&mut rmt, 0x400 + 64 * 4, (1 << 15) | 10 | (10 << 16));
        write32(&mut rmt, 0x400 + 65 * 4, 0);
        write32(&mut rmt, 0x58, 1 << 3); // INT_ENA: CH1_TX_END
        (rmt, clock)
    }

    fn tick_at(rmt: &mut Esp32Rmt, bus: &mut crate::bus::SystemBus, clock: &CycleClock, at: u64) {
        clock.publish(at);
        rmt.tick_with_bus(bus);
    }

    #[test]
    fn s2_registers_sit_at_the_s2_offsets() {
        let mut rmt = Esp32Rmt::with_layout(RmtLayout::ESP32S2, 50);
        assert_eq!(read32(&rmt, 0x10), 0x3910_0002, "CH0CONF0 reset");
        assert_eq!(read32(&rmt, 0x14), 0x0000_0F20, "CH0CONF1 reset");
        assert_eq!(read32(&rmt, 0xFC), 0x1907_2601, "RMT_DATE reset");
        write32(&mut rmt, 0x80, 0x3); // APB_CONF
        assert_eq!(read32(&rmt, 0x80), 0x3);
        write32(&mut rmt, 0x7C, 0x55); // CH3_TX_LIM
        assert_eq!(read32(&rmt, 0x7C), 0x55);
        // RMTMEM: ch3's last word is the last word of the 256-word block.
        write32(&mut rmt, 0x7FC, 0xDEAD_BEEF);
        assert_eq!(read32(&rmt, 0x7FC), 0xDEAD_BEEF);
        assert_eq!(read32(&rmt, 0x800), 0, "nothing past ch3's RMTMEM");
        // The classic offsets are not S2 registers.
        write32(&mut rmt, 0xF0, 0x3);
        assert_eq!(read32(&rmt, 0xF0), 0);
    }

    #[test]
    fn tx_end_is_bit_3n_and_lands_when_the_stream_has_played() {
        let (mut rmt, clock) = s2_with_stream();
        let mut bus = crate::bus::SystemBus::new();
        clock.publish(1_000);
        write32(&mut rmt, 0x1C, 1); // CH1CONF1: TX_START
        assert_eq!(read32(&rmt, 0x1C) & 1, 0, "TX_START self-clears");
        tick_at(&mut rmt, &mut bus, &clock, 1_000);
        tick_at(&mut rmt, &mut bus, &clock, 1_058);
        assert_eq!(read32(&rmt, 0x50), 0, "59 of 60 cycles played");
        tick_at(&mut rmt, &mut bus, &clock, 1_059);
        assert_eq!(read32(&rmt, 0x50), 1 << 3, "CH1_TX_END, not bit 1");
        assert_eq!(read32(&rmt, 0x54), 1 << 3, "INT_ST");
        write32(&mut rmt, 0x5C, 1 << 3); // INT_CLR
        assert_eq!(read32(&rmt, 0x50), 0);
        assert_eq!(rmt.tx_start_count, 1);
    }

    #[test]
    fn a_later_stream_is_timed_from_its_own_start() {
        let (mut rmt, clock) = s2_with_stream();
        let mut bus = crate::bus::SystemBus::new();
        clock.publish(1_000);
        write32(&mut rmt, 0x1C, 1);
        tick_at(&mut rmt, &mut bus, &clock, 1_000);
        tick_at(&mut rmt, &mut bus, &clock, 1_100);
        write32(&mut rmt, 0x5C, 1 << 3);
        // A second transmission long after the first ended.
        clock.publish(500_000);
        write32(&mut rmt, 0x1C, 1);
        tick_at(&mut rmt, &mut bus, &clock, 500_000);
        assert!(rmt.lines.level(1), "the high half-symbol is on the wire");
        assert_eq!(read32(&rmt, 0x50), 0, "not done on its first tick");
        tick_at(&mut rmt, &mut bus, &clock, 500_030);
        assert!(!rmt.lines.level(1), "low half after 10 ticks");
        tick_at(&mut rmt, &mut bus, &clock, 500_059);
        assert_eq!(read32(&rmt, 0x50), 1 << 3);
    }

    #[test]
    fn classic_keeps_its_offsets_and_eight_channels() {
        let mut rmt = Esp32Rmt::new(47);
        assert_eq!(read32(&rmt, 0x20), 0x3110_0002, "CH0CONF0 reset");
        write32(&mut rmt, 0xF0, 0x3);
        assert_eq!(read32(&rmt, 0xF0), 0x3, "APB_CONF");
        write32(&mut rmt, 0xFFC, 0x1234); // ch7's last RMTMEM word
        assert_eq!(read32(&rmt, 0xFFC), 0x1234);
        assert_eq!(rmt.lines.names().len(), 8);
    }
}
