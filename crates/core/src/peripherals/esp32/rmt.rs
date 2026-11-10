// LabWired - Firmware Simulation Platform
//! Classic ESP32 (LX6) RMT — TX → GPIO pad drive for WS2812 / NeoPixel.
//!
//! Port of the ESP32-S3 RMT Stage-2 engine to the classic register map
//! (TRM v4.6 / `soc/rmt_struct.h`): eight TX/RX-capable channels, CONF0/CONF1
//! pairs, RMTMEM at offset 0x800 (64 words × 8), and `CHn_TX_END` at bit
//! `channel * 3`. Adafruit NeoPixel on Arduino-ESP32 3.x programs this block
//! through IDF5 `rmt_transmit` (copy encoder + ping-pong on TX_THR).
//!
//! Scope: TX playback that reads RMTMEM symbol-by-symbol, drives every pad
//! whose `FUNCn_OUT_SEL` selects `RMT_SIG_OUTn`, raises `TX_THR` so the IDF
//! ISR can refill, and latches `TX_END` when an END marker (duration 0) is
//! seen. RX, carrier, and loop modes are out of scope.
//! STATUS and APB address progress fields remain reset-zero placeholders;
//! their read-only access does not claim full status-machine modeling.

use crate::peripherals::esp32::gpio::{
    RMT_SIG_OUT0, RMT_SIG_OUT1, RMT_SIG_OUT2, RMT_SIG_OUT3, RMT_SIG_OUT4, RMT_SIG_OUT5,
    RMT_SIG_OUT6, RMT_SIG_OUT7,
};
use crate::{Bus, CycleClock, Peripheral, PeripheralTickResult, SimResult};

/// Channels on the classic RMT (all TX/RX capable).
const CHANNELS: usize = 8;
/// Words per memory block / default per-channel window.
const RMT_BLOCK_WORDS: usize = 64;
/// Total RMTMEM depth: 8 × 64.
const RMT_MEM_WORDS: usize = RMT_BLOCK_WORDS * CHANNELS;

/// Direct RMTMEM aperture (`RMT_CHANNEL_MEM(0)` = base + 0x800).
const RMTMEM_START: u64 = 0x800;
const RMTMEM_END: u64 = RMTMEM_START + (RMT_MEM_WORDS as u64) * 4;

/// APB-conf at 0xF0: bit 0 = fifo_mask (1 → RMTMEM access, not APB FIFO).
/// APB-conf bit 1 = mem_tx_wrap_en.
const APB_CONF_MEM_TX_WRAP: u32 = 1 << 1;

/// CONF1 TX_START (WT, self-clearing).
pub const TX_START_BIT: u32 = 1 << 0;
/// CONF1 write-trigger bits that must not stick on readback.
const CONF1_WT_MASK: u32 = (1 << 0) // tx_start
    | (1 << 2) // mem_wr_rst
    | (1 << 3) // mem_rd_rst
    | (1 << 4) // apb_mem_rst
    | (1 << 10); // ref_cnt_rst

/// Symbol duration field (15 bits).
const ENTRY_DUR_MASK: u32 = 0x7FFF;

/// Valid INT bits: 8×(TX_END|RX_END|ERR) + 8×TX_THR = 32 bits used.
const INT_VALID_MASK: u32 = 0xFFFF_FFFF;

const fn tx_end_bit(ch: usize) -> u32 {
    1 << (ch * 3)
}
const fn tx_thr_bit(ch: usize) -> u32 {
    1 << (24 + ch)
}

const fn rmt_out_signal(ch: usize) -> u32 {
    match ch {
        0 => RMT_SIG_OUT0,
        1 => RMT_SIG_OUT1,
        2 => RMT_SIG_OUT2,
        3 => RMT_SIG_OUT3,
        4 => RMT_SIG_OUT4,
        5 => RMT_SIG_OUT5,
        6 => RMT_SIG_OUT6,
        _ => RMT_SIG_OUT7,
    }
}

/// Streaming TX playback: reads RMTMEM one symbol at a time so IDF ping-pong
/// refills (TX_THR ISR) become visible before the next half wraps.
#[derive(Debug)]
struct TxPlayback {
    ch: usize,
    signal: u32,
    pads: Vec<u8>,
    resolved: bool,
    /// Next word index within the channel's mem window.
    mem_idx: usize,
    /// Symbols consumed since TX_START (TX_THR threshold).
    symbols_sent: usize,
    /// Last driven level (idle low).
    level: bool,
    /// Remaining sim cycles of the current pulse half.
    wait: u64,
    /// 0 = playing duration0, 1 = playing duration1, 2 = need next symbol.
    half: u8,
    current: u32,
    /// True once an END marker ended the stream.
    finished: bool,
    /// TX_THR already raised for the current limit crossing.
    thr_armed_at: Option<usize>,
}

#[derive(Debug)]
pub struct Esp32Rmt {
    source_id: u32,
    conf0: [u32; CHANNELS],
    conf1: [u32; CHANNELS],
    status: [u32; CHANNELS],
    apb_mem_addr: [u32; CHANNELS],
    carrier_duty: [u32; CHANNELS],
    tx_lim: [u32; CHANNELS],
    mem: [u32; RMT_MEM_WORDS],
    ch_wr: [usize; CHANNELS],
    int_raw: u32,
    int_ena: u32,
    apb_conf: u32,
    date: u32,
    playback: Option<TxPlayback>,
    /// Hold TX-done level IRQ so IDF can reach WaitBits (S3 pattern).
    irq_holdoff_cycles: u32,
    tx_start_count: u32,
    clock: Option<CycleClock>,
    last_bus_tick_cycle: Option<u64>,
    pending_events: Vec<(u64, u32)>,
}

const RMT_HOLDOFF_TOKEN: u32 = 1;

impl Esp32Rmt {
    crate::cycle_clock::scheduler_mode!();

    /// `source_id` = `ETS_RMT_INTR_SOURCE` (= 47 on classic ESP32).
    pub fn new(source_id: u32) -> Self {
        // CONF0 reset: div_cnt=1 is friendlier than 0(=256) for WS2812 tests;
        // silicon reset is often 0 — IDF always programs DIV before TX_START.
        let conf0 = [0x1u32; CHANNELS]; // div=1, mem_size=0 → treated as 1 block
        Self {
            source_id,
            conf0,
            conf1: [0; CHANNELS],
            status: [0; CHANNELS],
            apb_mem_addr: [0; CHANNELS],
            carrier_duty: [0x0040_0040; CHANNELS],
            tx_lim: [0; CHANNELS],
            mem: [0; RMT_MEM_WORDS],
            ch_wr: [0; CHANNELS],
            int_raw: 0,
            int_ena: 0,
            apb_conf: 0,
            date: 0x1602_2600, // vendor SVD: 32-bit R/W version register
            playback: None,
            irq_holdoff_cycles: 0,
            tx_start_count: 0,
            clock: None,
            last_bus_tick_cycle: None,
            pending_events: Vec::new(),
        }
    }

    pub fn tx_start_count(&self) -> u32 {
        self.tx_start_count
    }

    /// Sim cycles to hold off TX_END level IRQ after the stream ends.
    const TX_IRQ_HOLDOFF: u32 = 2000;

    fn int_st(&self) -> u32 {
        self.int_raw & self.int_ena & INT_VALID_MASK
    }

    fn mem_size_blocks(&self, ch: usize) -> usize {
        let n = ((self.conf0[ch] >> 24) & 0xF) as usize;
        if n == 0 {
            1
        } else {
            n
        }
    }

    fn mem_window_words(&self, ch: usize) -> usize {
        let base = ch * RMT_BLOCK_WORDS;
        (self.mem_size_blocks(ch) * RMT_BLOCK_WORDS).min(RMT_MEM_WORDS.saturating_sub(base))
    }

    fn div_cnt(&self, ch: usize) -> u64 {
        let d = (self.conf0[ch] & 0xFF) as u64;
        if d == 0 {
            256
        } else {
            d
        }
    }

    fn cycles_per_rmt_tick(&self, ch: usize) -> u64 {
        self.div_cnt(ch)
    }

    fn wrap_enabled(&self) -> bool {
        self.apb_conf & APB_CONF_MEM_TX_WRAP != 0
    }

    fn conf_index(offset: u64) -> Option<(usize, bool)> {
        if (0x20..0x60).contains(&offset) {
            let rel = offset - 0x20;
            let ch = (rel / 8) as usize;
            if ch < CHANNELS {
                return Some((ch, rel % 8 == 0)); // true = CONF0
            }
        }
        None
    }

    fn rmtmem_index(offset: u64) -> Option<usize> {
        if (RMTMEM_START..RMTMEM_END).contains(&offset) && offset % 4 == 0 {
            Some(((offset - RMTMEM_START) / 4) as usize)
        } else {
            None
        }
    }

    fn chndata_index(offset: u64) -> Option<usize> {
        if offset < 0x20 && offset % 4 == 0 {
            let ch = (offset / 4) as usize;
            if ch < CHANNELS {
                return Some(ch);
            }
        }
        None
    }

    fn arm_tx(&mut self, ch: usize) {
        self.playback = Some(TxPlayback {
            ch,
            signal: rmt_out_signal(ch),
            pads: Vec::new(),
            resolved: false,
            mem_idx: 0,
            symbols_sent: 0,
            level: false,
            wait: 0,
            half: 2, // fetch first symbol immediately
            current: 0,
            finished: false,
            thr_armed_at: None,
        });
    }

    /// Reach the sibling GPIO through the bus's named-peripheral capability
    /// so pad lookup / drive does not add `as_any_mut` / `downcast_mut` sites.
    fn with_gpio<R>(bus: &mut dyn Bus, f: impl FnOnce(&mut dyn Peripheral) -> R) -> Option<R> {
        let mut out = None;
        let mut f = Some(f);
        bus.with_peripheral_mut("gpio", &mut |p| {
            if let Some(cb) = f.take() {
                out = Some(cb(p));
            }
        });
        out
    }

    fn load_next_symbol(pb: &mut TxPlayback, mem: &[u32], window: usize, wrap: bool) -> bool {
        if window == 0 {
            pb.finished = true;
            return false;
        }
        if pb.mem_idx >= window {
            if wrap {
                pb.mem_idx = 0;
            } else {
                pb.finished = true;
                return false;
            }
        }
        let base = pb.ch * RMT_BLOCK_WORDS;
        pb.current = mem[base + pb.mem_idx];
        pb.mem_idx += 1;
        pb.symbols_sent = pb.symbols_sent.saturating_add(1);
        pb.half = 0;
        true
    }

    fn pulse_parts(word: u32) -> [(u64, bool); 2] {
        [
            ((word & ENTRY_DUR_MASK) as u64, (word >> 15) & 1 != 0),
            (
                ((word >> 16) & ENTRY_DUR_MASK) as u64,
                (word >> 31) & 1 != 0,
            ),
        ]
    }

    fn read_word(&self, offset: u64) -> u32 {
        if let Some((ch, is_conf0)) = Self::conf_index(offset) {
            return if is_conf0 {
                self.conf0[ch]
            } else {
                self.conf1[ch]
            };
        }
        if let Some(ch) = Self::chndata_index(offset) {
            return if self.ch_wr[ch] == 0 {
                0
            } else {
                let len = self.mem_window_words(ch);
                self.mem[ch * RMT_BLOCK_WORDS + (self.ch_wr[ch] - 1) % len.max(1)]
            };
        }
        if let Some(i) = Self::rmtmem_index(offset) {
            return self.mem[i];
        }
        if (0x60..0x80).contains(&offset) && offset % 4 == 0 {
            let ch = ((offset - 0x60) / 4) as usize;
            if ch < CHANNELS {
                return self.status[ch];
            }
        }
        if (0x80..0xA0).contains(&offset) && offset % 4 == 0 {
            let ch = ((offset - 0x80) / 4) as usize;
            if ch < CHANNELS {
                return self.apb_mem_addr[ch];
            }
        }
        if (0xB0..0xD0).contains(&offset) && offset % 4 == 0 {
            let ch = ((offset - 0xB0) / 4) as usize;
            if ch < CHANNELS {
                return self.carrier_duty[ch];
            }
        }
        if (0xD0..0xF0).contains(&offset) && offset % 4 == 0 {
            let ch = ((offset - 0xD0) / 4) as usize;
            if ch < CHANNELS {
                return self.tx_lim[ch];
            }
        }
        match offset {
            0xA0 => self.int_raw,
            0xA4 => self.int_st(),
            0xA8 => self.int_ena,
            0xAC => 0, // INT_CLR W1C
            0xF0 => self.apb_conf,
            0xFC => self.date,
            _ => {
                crate::census_reg!("esp32.rmt:Esp32Rmt", offset, "read");
                0
            }
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if let Some((ch, is_conf0)) = Self::conf_index(offset) {
            if is_conf0 {
                self.conf0[ch] = value;
            } else {
                let start = value & TX_START_BIT != 0;
                self.conf1[ch] = value & !CONF1_WT_MASK;
                if start {
                    // Do NOT latch TX_END yet — wait for END marker / underrun
                    // so IDF ping-pong (TX_THR refills) can run to completion.
                    self.tx_start_count = self.tx_start_count.saturating_add(1);
                    self.last_bus_tick_cycle = self.clock.as_ref().map(|c| c.now());
                    self.arm_tx(ch);
                }
            }
            return;
        }
        if let Some(ch) = Self::chndata_index(offset) {
            let len = self.mem_window_words(ch).max(1);
            let slot = ch * RMT_BLOCK_WORDS + self.ch_wr[ch] % len;
            self.mem[slot] = value;
            self.ch_wr[ch] = self.ch_wr[ch].wrapping_add(1);
            return;
        }
        if let Some(i) = Self::rmtmem_index(offset) {
            self.mem[i] = value;
            return;
        }
        if (0xB0..0xD0).contains(&offset) && offset % 4 == 0 {
            let ch = ((offset - 0xB0) / 4) as usize;
            if ch < CHANNELS {
                self.carrier_duty[ch] = value;
                return;
            }
        }
        if (0xD0..0xF0).contains(&offset) && offset % 4 == 0 {
            let ch = ((offset - 0xD0) / 4) as usize;
            if ch < CHANNELS {
                self.tx_lim[ch] = value & 0x1FF;
                return;
            }
        }
        match offset {
            0xA0 | 0xA4 => {} // INT_RAW and INT_ST are hardware-owned (RO).
            0xA8 => self.int_ena = value & INT_VALID_MASK,
            0xAC => self.int_raw &= !(value & INT_VALID_MASK),
            0xF0 => self.apb_conf = value,
            0xFC => self.date = value,
            _ => {
                crate::census_reg!("esp32.rmt:Esp32Rmt", offset, "write");
            }
        }
    }

    fn complete_tx(&mut self, ch: usize) {
        self.int_raw |= tx_end_bit(ch);
        self.irq_holdoff_cycles = Self::TX_IRQ_HOLDOFF;
        self.pending_events.push((
            u64::from(Self::TX_IRQ_HOLDOFF.saturating_sub(1)),
            RMT_HOLDOFF_TOKEN,
        ));
        self.playback = None;
    }
}

impl Peripheral for Esp32Rmt {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        if self.irq_holdoff_cycles > 0 {
            self.irq_holdoff_cycles = self.irq_holdoff_cycles.saturating_sub(1);
        }
        let explicit_irqs = if self.irq_holdoff_cycles == 0 && self.int_st() != 0 {
            Some(vec![self.source_id])
        } else {
            None
        };
        PeripheralTickResult {
            explicit_irqs,
            ..PeripheralTickResult::default()
        }
    }

    fn needs_bus_tick(&self) -> bool {
        self.playback.is_some() || self.irq_holdoff_cycles > 0
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn idle_poll_bus_tick(&self) -> bool {
        // Playback and TX_END holdoff must still advance when the guest is
        // idle-fast-forwarding: uses_scheduler() removes this model from the
        // legacy walk, and arm_tx does not otherwise get a guaranteed tick.
        self.needs_bus_tick()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        // TX_THR must be visible immediately (ping-pong refill); TX_END respects
        // holdoff so the waveform can finish before the done ISR runs.
        let thr_pending = (0..CHANNELS)
            .any(|ch| self.int_raw & tx_thr_bit(ch) != 0 && self.int_ena & tx_thr_bit(ch) != 0);
        let end_pending = self.irq_holdoff_cycles == 0 && self.int_st() != 0;
        if thr_pending || end_pending {
            out.push(self.source_id);
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.pending_events)
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn Bus,
    ) -> crate::sched::EventResult {
        let now = sched.now();
        if let Some(last) = self.last_bus_tick_cycle {
            if now > last && self.irq_holdoff_cycles > 0 {
                self.irq_holdoff_cycles = self
                    .irq_holdoff_cycles
                    .saturating_sub(u32::try_from(now - last).unwrap_or(u32::MAX));
            }
        }
        crate::sched::EventResult::default()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        let now = self.clock.as_ref().map(|c| c.now());
        let elapsed = match (self.last_bus_tick_cycle, now) {
            (Some(last), Some(now)) if now > last => now - last,
            _ => 1,
        };
        if let Some(now) = now {
            self.last_bus_tick_cycle = Some(now);
        }
        if self.irq_holdoff_cycles > 0 {
            self.irq_holdoff_cycles = self
                .irq_holdoff_cycles
                .saturating_sub(u32::try_from(elapsed).unwrap_or(u32::MAX));
        }
        if self.playback.is_none() {
            return;
        }

        // Resolve pads once.
        if !self.playback.as_ref().unwrap().resolved {
            let signal = self.playback.as_ref().unwrap().signal;
            let pads =
                Self::with_gpio(bus, |g| g.pads_for_output_signal(signal)).unwrap_or_default();
            let pb = self.playback.as_mut().unwrap();
            pb.resolved = true;
            pb.pads = pads;
            if pb.pads.is_empty() {
                let ch = pb.ch;
                self.complete_tx(ch);
                return;
            }
        }

        let ch = self.playback.as_ref().unwrap().ch;
        let window = self.mem_window_words(ch);
        let wrap = self.wrap_enabled();
        let per_tick = self.cycles_per_rmt_tick(ch);
        let lim = (self.tx_lim[ch] & 0x1FF) as usize;
        let bus_cycle = now.unwrap_or(0);

        let mut edge_levels: Vec<bool> = Vec::new();
        let mut done_ch: Option<usize> = None;
        let mut raise_thr = false;

        {
            let pb = self.playback.as_mut().unwrap();
            let mut left = elapsed;
            while left > 0 && !pb.finished {
                if pb.half == 2 {
                    if !Self::load_next_symbol(pb, &self.mem, window, wrap) {
                        break;
                    }
                    // TX_THR when we cross the programmed limit (IDF ping-pong).
                    if lim > 0
                        && pb.symbols_sent >= lim
                        && pb.thr_armed_at != Some(pb.symbols_sent / lim)
                    {
                        pb.thr_armed_at = Some(pb.symbols_sent / lim);
                        raise_thr = true;
                    }
                    let parts = Self::pulse_parts(pb.current);
                    // END marker: duration0 == 0
                    if parts[0].0 == 0 {
                        pb.finished = true;
                        break;
                    }
                }

                let parts = Self::pulse_parts(pb.current);
                let (dur, lvl) = parts[pb.half as usize];
                if dur == 0 {
                    // Trailing empty half — advance / finish
                    if pb.half == 0 {
                        pb.half = 1;
                        continue;
                    }
                    pb.half = 2;
                    continue;
                }
                let pulse_cycles = dur.saturating_mul(per_tick);
                if pb.wait == 0 {
                    // Starting this half: emit level change if needed
                    if lvl != pb.level {
                        pb.level = lvl;
                        edge_levels.push(lvl);
                    }
                    pb.wait = pulse_cycles;
                }
                if left < pb.wait {
                    pb.wait -= left;
                    left = 0;
                } else {
                    left -= pb.wait;
                    pb.wait = 0;
                    if pb.half == 0 {
                        pb.half = 1;
                    } else {
                        pb.half = 2;
                    }
                }
            }
            if pb.finished {
                done_ch = Some(pb.ch);
            }
        }

        if raise_thr {
            self.int_raw |= tx_thr_bit(ch);
        }

        if !edge_levels.is_empty() {
            let pads = self.playback.as_ref().unwrap().pads.clone();
            Self::with_gpio(bus, |g| {
                g.sync_observer_cycle(bus_cycle);
                for level in edge_levels {
                    for &pad in &pads {
                        g.drive_pad_output(pad, level);
                    }
                }
            });
        }

        if let Some(ch) = done_ch {
            self.complete_tx(ch);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf1_tx_start_arms_playback_and_rmtmem_roundtrips() {
        let mut r = Esp32Rmt::new(47);
        // mem_size=2 blocks, div=1
        r.write_word(0x20, (2 << 24) | 1);
        r.mem[0] = 0x0002_8002; // high 2, low 2
        r.mem[1] = 0; // END
        r.write_word(0x24, TX_START_BIT);
        assert!(r.playback.is_some());
        assert_eq!(r.tx_start_count(), 1);
        assert_eq!(r.read_word(RMTMEM_START), 0x0002_8002);
    }
}
