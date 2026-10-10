// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! i.MX RT1060 Synchronous Audio Interface (SAI1 `0x4038_4000`, SAI2
//! `0x4038_8000`, SAI3 `0x4038_C000`, IMXRT1060RM §38).
//!
//! Transmitter and receiver each have a 32-word FIFO per data line
//! (`PARAM` = 0x0005_0504) and run in frames: every frame consumes (TX) or
//! produces (RX) one word per unmasked word slot (`FRSZ`+1 slots, `xMR`
//! mask) on every enabled data line (`TCR3.TCE` / `RCR3.RCE`). The frame
//! period is derived from the bit clock the registers program:
//! BCLK = MCLK / ((DIV+1) * 2) when the bit clock is generated internally
//! (`BCD` = 1), and one frame = W0W+1 + FRSZ*(WNW+1) bit clocks. A
//! synchronous side (`SYNC` = 1) runs off the other side's clock.
//!
//! Flags per side: `FRF` (TX: FIFO at or below the watermark, RX: above
//! it), `FWF` (TX empty / RX full), `FEF` (TX underrun / RX overrun,
//! write-1-to-clear), `WSF` (start of frame, w1c). The interrupt line and
//! the DMA requests (`FRDE`/`FWDE`) follow them. Transmitted words are kept
//! in a bounded capture per data line (what a codec's DAC would receive).
//! Received words come from a queued source, or from silence. A test script
//! can instead drive line 0: `rx_amp` is the peak sample and `rx_hz` is a
//! tone (0 Hz holds `rx_amp` as a constant). Slot 0 carries that sample.
//! The other slots on that line stay 0. Logs: `rx` (`words N amp A hz H`)
//! and `tx` (`words N peak P tail T`). `peak` is the largest absolute
//! sample seen. `tail` is that peak over the last 256 transmitted words.
//!
//! WAV files, for listening to a run: `rx_wav` (config) plays a PCM or
//! float WAV into line 0 slot 0, mixed to mono, resampled to the frame
//! rate and scaled by `rx_wav_gain`; it overrides `rx_amp` and is silence
//! after its end. `tx_wav` writes slots 0 and 1 of TX line 0 as a 16-bit
//! stereo WAV at the frame rate. The file is rewritten about once per
//! second of device audio and when the model is dropped.

use super::{byte_of, Timebase};
use crate::sim_input::{InputChannel, SimInput, SimInputError};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;

const TAIL_WORDS: usize = 256;

const RX_INPUTS: &[InputChannel] = &[
    InputChannel {
        key: std::borrow::Cow::Borrowed("rx_amp"),
        label: std::borrow::Cow::Borrowed("Receive amplitude"),
        unit: std::borrow::Cow::Borrowed("lsb"),
        min: -32767.0,
        max: 32767.0,
        default: None,
    },
    InputChannel {
        key: std::borrow::Cow::Borrowed("rx_hz"),
        label: std::borrow::Cow::Borrowed("Receive tone"),
        unit: std::borrow::Cow::Borrowed("Hz"),
        min: 0.0,
        max: 20_000.0,
        default: None,
    },
];

const FIFO: usize = 32;
const LINES: usize = 4;
const CAPTURE: usize = 1 << 16;

const XCSR_FRDE: u32 = 1 << 0;
const XCSR_FWDE: u32 = 1 << 1;
const XCSR_FRIE: u32 = 1 << 8;
const XCSR_FWIE: u32 = 1 << 9;
const XCSR_FEIE: u32 = 1 << 10;
const XCSR_SEIE: u32 = 1 << 11;
const XCSR_WSIE: u32 = 1 << 12;
const XCSR_FRF: u32 = 1 << 16;
const XCSR_FWF: u32 = 1 << 17;
const XCSR_FEF: u32 = 1 << 18;
const XCSR_SEF: u32 = 1 << 19;
const XCSR_WSF: u32 = 1 << 20;
const XCSR_SR: u32 = 1 << 24;
const XCSR_FR: u32 = 1 << 25;
const XCSR_EN: u32 = 1 << 31;

#[derive(Debug, Default)]
struct Side {
    csr: u32,
    cr: [u32; 5], // xCR1..xCR5
    mr: u32,
    fifo: [VecDeque<u32>; LINES],
    /// Cycle the side was enabled / frames processed since.
    t0: u64,
    frames: u64,
    /// Sticky flags: FEF, SEF, WSF.
    sticky: u32,
}

impl Side {
    fn enabled(&self) -> bool {
        self.csr & XCSR_EN != 0
    }
    fn lines(&self) -> u32 {
        (self.cr[2] >> 16) & 0xF
    }
    fn watermark(&self) -> usize {
        (self.cr[0] & 0x1F) as usize
    }
    fn slots(&self) -> u32 {
        ((self.cr[3] >> 16) & 0x1F) + 1
    }
    fn first_line(&self) -> usize {
        (0..LINES)
            .find(|&l| self.lines() & (1 << l) != 0)
            .unwrap_or(0)
    }
}

#[derive(Debug)]
struct Inner {
    tx: Side,
    rx: Side,
    /// TX capture per line (bounded), and the total count.
    captured: [VecDeque<u32>; LINES],
    tx_words: u64,
    /// RX sample source per line.
    rx_source: [VecDeque<u32>; LINES],
    rx_words: u64,
    /// Line 0 slot 0 source. `rx_amp` 0 keeps the queue (silence when empty).
    /// `rx_hz` 0 holds `rx_amp`. A positive `rx_hz` is a sine of that peak.
    rx_amp: i32,
    rx_hz: f64,
    rx_phase: f64,
    /// Largest absolute low-16 sample transmitted on any line.
    tx_peak: u32,
    rx_wav: Option<RxWav>,
    tx_wav: Option<TxWav>,
}

/// A WAV played into the receiver (mono, -1..1, at its own rate).
#[derive(Debug)]
struct RxWav {
    samples: Vec<f32>,
    rate: f64,
    gain: f64,
    /// Position in source samples.
    pos: f64,
}

/// TX line 0 slots 0 and 1, written out as a WAV.
#[derive(Debug)]
struct TxWav {
    path: std::path::PathBuf,
    rate: u32,
    frames: Vec<[i16; 2]>,
    cur: [i16; 2],
    flushed: usize,
}

impl TxWav {
    fn flush(&mut self) {
        if self.frames.len() == self.flushed {
            return;
        }
        let rate = self.rate.max(1);
        let data = (self.frames.len() * 4) as u32;
        let mut b = Vec::with_capacity(44 + data as usize);
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for f in &self.frames {
            b.extend_from_slice(&f[0].to_le_bytes());
            b.extend_from_slice(&f[1].to_le_bytes());
        }
        if let Err(e) = std::fs::write(&self.path, b) {
            tracing::warn!("imxrt_sai: tx_wav {}: {e}", self.path.display());
        }
        self.flushed = self.frames.len();
    }
}

/// Read a RIFF WAV (PCM 8/16/24/32-bit or IEEE float 32/64) as mono -1..1.
fn read_wav(path: &std::path::Path) -> Result<(Vec<f32>, f64), String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(format!("{}: not a RIFF WAVE file", path.display()));
    }
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    let mut o = 12;
    while o + 8 <= b.len() {
        let id = &b[o..o + 4];
        let len = u32_at(o + 4) as usize;
        let body = &b[o + 8..(o + 8 + len).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            let mut tag = u16_at(o + 8);
            if tag == 0xFFFE && body.len() >= 26 {
                tag = u16_at(o + 8 + 24); // WAVE_FORMAT_EXTENSIBLE sub-format
            }
            fmt = Some((tag, u16_at(o + 10), u32_at(o + 12), u16_at(o + 22)));
        } else if id == b"data" {
            data = Some(body);
        }
        o += 8 + len + (len & 1);
    }
    let (tag, ch, rate, bits) = fmt.ok_or_else(|| format!("{}: no fmt chunk", path.display()))?;
    let data = data.ok_or_else(|| format!("{}: no data chunk", path.display()))?;
    let ch = ch.max(1) as usize;
    let width = (bits as usize).div_ceil(8);
    let sample = |s: &[u8]| -> Option<f32> {
        Some(match (tag, bits) {
            (1, 8) => (s[0] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
            (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            (3, 64) => f64::from_le_bytes(s[..8].try_into().ok()?) as f32,
            _ => return None,
        })
    };
    let frame = width * ch;
    let mut out = Vec::with_capacity(data.len() / frame.max(1));
    for f in data.chunks_exact(frame.max(1)) {
        let mut sum = 0.0f32;
        for c in f.chunks_exact(width) {
            sum += sample(c).ok_or_else(|| {
                format!("{}: unsupported format {tag} / {bits}-bit", path.display())
            })?;
        }
        out.push(sum / ch as f32);
    }
    Ok((out, rate as f64))
}

#[derive(Debug)]
pub struct ImxrtSai {
    inner: RefCell<Inner>,
    time: Timebase,
    mclk_hz: u64,
    /// SAI3 has separate NVIC lines: the entry's line carries the receiver
    /// and this one the transmitter (SAI3_TX = 59). `None`: one combined
    /// line (SAI1, SAI2).
    tx_irq: Option<u32>,
    tx_level: std::cell::Cell<bool>,
}

/// SAI master clock when the chip YAML does not say otherwise:
/// 256 x 44.1 kHz, the FB200's sample rate.
pub const DEFAULT_MCLK_HZ: u64 = 11_289_600;

impl Default for ImxrtSai {
    fn default() -> Self {
        Self::new(DEFAULT_MCLK_HZ)
    }
}

impl ImxrtSai {
    pub fn new(mclk_hz: u64) -> Self {
        Self {
            inner: RefCell::new(Inner {
                tx: Side::default(),
                rx: Side::default(),
                captured: Default::default(),
                tx_words: 0,
                rx_source: Default::default(),
                rx_words: 0,
                rx_amp: 0,
                rx_hz: 0.0,
                rx_phase: 0.0,
                tx_peak: 0,
                rx_wav: None,
                tx_wav: None,
            }),
            time: Timebase::default(),
            mclk_hz: mclk_hz.max(1),
            tx_irq: None,
            tx_level: std::cell::Cell::new(false),
        }
    }

    /// Give the transmitter its own NVIC line (SAI3).
    pub fn with_tx_irq(mut self, line: u32) -> Self {
        self.tx_irq = Some(line);
        self
    }

    /// Play `path` into line 0 slot 0 (see the module docs).
    pub fn with_rx_wav(self, path: &std::path::Path, gain: f64) -> Result<Self, String> {
        let (samples, rate) = read_wav(path)?;
        self.inner.borrow_mut().rx_wav = Some(RxWav {
            samples,
            rate,
            gain,
            pos: 0.0,
        });
        Ok(self)
    }

    /// Write TX line 0 slots 0 and 1 to `path` (see the module docs).
    pub fn with_tx_wav(self, path: &std::path::Path) -> Self {
        self.inner.borrow_mut().tx_wav = Some(TxWav {
            path: path.to_path_buf(),
            rate: 0,
            frames: Vec::new(),
            cur: [0; 2],
            flushed: 0,
        });
        self
    }

    /// Write the `tx_wav` file now.
    pub fn flush_tx_wav(&self) {
        if let Some(w) = self.inner.borrow_mut().tx_wav.as_mut() {
            w.flush();
        }
    }

    /// Frame rate of `side` in Hz, from its bit clock (not rounded to cycles).
    fn frame_hz(&self, own: &Side, other: &Side) -> Option<u32> {
        let sync = (own.cr[1] >> 30) & 0x3;
        let clk = if sync == 1 { other } else { own };
        let cr2 = clk.cr[1];
        if cr2 & (1 << 24) == 0 {
            return None;
        }
        let bclk_hz = self.mclk_hz / (((cr2 & 0xFF) as u64 + 1) * 2);
        let w0 = ((own.cr[4] >> 16) & 0x1F) as u64 + 1;
        let wn = ((own.cr[4] >> 24) & 0x1F) as u64 + 1;
        let bits = w0 + (own.slots() as u64 - 1) * wn;
        Some((bclk_hz as f64 / bits as f64).round() as u32)
    }

    /// Words transmitted so far (all lines) and the recent capture of `line`.
    pub fn tx_capture(&self, line: usize) -> (u64, Vec<u32>) {
        let i = self.inner.borrow();
        (
            i.tx_words,
            i.captured
                .get(line)
                .map(|q| q.iter().copied().collect())
                .unwrap_or_default(),
        )
    }

    /// Frames each side has clocked so far: (tx, rx).
    pub fn frames(&self) -> (u64, u64) {
        let i = self.inner.borrow();
        (i.tx.frames, i.rx.frames)
    }

    /// Queue words the codec will drive into the receiver on `line`.
    pub fn push_rx_words(&mut self, line: usize, words: &[u32]) {
        if let Some(q) = self.inner.get_mut().rx_source.get_mut(line) {
            q.extend(words.iter().copied());
        }
    }

    /// Core cycles per frame of `side` (clocked from `clock_side` when
    /// synchronous), or None if no clock runs.
    fn frame_cycles(&self, own: &Side, other: &Side) -> Option<u64> {
        let sync = (own.cr[1] >> 30) & 0x3;
        let clk = if sync == 1 { other } else { own };
        let cr2 = clk.cr[1];
        if cr2 & (1 << 24) == 0 {
            // External bit clock: the FB200 codec is a clock slave, so an
            // external clock never runs in this system.
            return None;
        }
        let div = (cr2 & 0xFF) as u64;
        let bclk_hz = self.mclk_hz / ((div + 1) * 2);
        let w0 = ((own.cr[4] >> 16) & 0x1F) as u64 + 1;
        let wn = ((own.cr[4] >> 24) & 0x1F) as u64 + 1;
        let bits = w0 + (own.slots() as u64 - 1) * wn;
        if bclk_hz == 0 {
            return None;
        }
        Some((bits * self.time.cpu_hz() / bclk_hz).max(1))
    }

    fn sync(&self) {
        let now = self.time.now();
        let cpu_hz = self.time.cpu_hz();
        let mut guard = self.inner.borrow_mut();
        let i = &mut *guard;
        // TX frames.
        if i.tx.enabled() {
            let tx_hz = self.frame_hz(&i.tx, &i.rx);
            if let (Some(w), Some(hz)) = (i.tx_wav.as_mut(), tx_hz) {
                if w.rate == 0 {
                    w.rate = hz;
                }
            }
            if let Some(fc) = self.frame_cycles(&i.tx, &i.rx) {
                let due = now.saturating_sub(i.tx.t0) / fc;
                let todo = due.saturating_sub(i.tx.frames).min(4096);
                for _ in 0..todo {
                    i.tx.frames += 1;
                    i.tx.sticky |= XCSR_WSF;
                    for slot in 0..i.tx.slots() {
                        if i.tx.mr & (1 << slot) != 0 {
                            continue;
                        }
                        for l in 0..LINES {
                            if i.tx.lines() & (1 << l) == 0 {
                                continue;
                            }
                            let w = match i.tx.fifo[l].pop_front() {
                                Some(w) => w,
                                None => {
                                    i.tx.sticky |= XCSR_FEF; // underrun
                                    0
                                }
                            };
                            i.tx_words += 1;
                            let sample = (w as u16) as i16;
                            i.tx_peak = i.tx_peak.max(u32::from(sample.unsigned_abs()));
                            if i.captured[l].len() >= CAPTURE {
                                i.captured[l].pop_front();
                            }
                            i.captured[l].push_back(w);
                            if l == 0 && slot < 2 {
                                if let Some(wav) = i.tx_wav.as_mut() {
                                    wav.cur[slot as usize] = sample;
                                }
                            }
                        }
                    }
                    if let Some(wav) = i.tx_wav.as_mut() {
                        wav.frames.push(wav.cur);
                        wav.cur = [0; 2];
                        if wav.frames.len() - wav.flushed >= wav.rate.max(1) as usize {
                            wav.flush();
                        }
                    }
                }
                if due > i.tx.frames + 4096 {
                    i.tx.frames = due; // far behind (debugger-style jump)
                }
            }
        }
        // RX frames.
        if i.rx.enabled() {
            if let Some(fc) = self.frame_cycles(&i.rx, &i.tx) {
                let due = now.saturating_sub(i.rx.t0) / fc;
                let todo = due.saturating_sub(i.rx.frames).min(4096);
                let rx_hz = self
                    .frame_hz(&i.rx, &i.tx)
                    .map_or(cpu_hz as f64 / fc as f64, f64::from);
                for _ in 0..todo {
                    i.rx.frames += 1;
                    i.rx.sticky |= XCSR_WSF;
                    for slot in 0..i.rx.slots() {
                        if i.rx.mr & (1 << slot) != 0 {
                            continue;
                        }
                        for l in 0..LINES {
                            if i.rx.lines() & (1 << l) == 0 {
                                continue;
                            }
                            let w = match (l, &i.rx_wav) {
                                (0, Some(wav)) if slot == 0 => wav_sample(wav),
                                (0, Some(_)) => 0,
                                _ => rx_sample(i, cpu_hz, fc, l, slot),
                            };
                            i.rx_words += 1;
                            if i.rx.fifo[l].len() >= FIFO {
                                i.rx.sticky |= XCSR_FEF; // overrun
                            } else {
                                i.rx.fifo[l].push_back(w);
                            }
                        }
                    }
                    if let Some(wav) = i.rx_wav.as_mut() {
                        wav.pos += wav.rate / rx_hz;
                    }
                }
                if due > i.rx.frames + 4096 {
                    i.rx.frames = due;
                }
            }
        }
    }

    fn csr_view(side: &Side, tx: bool) -> u32 {
        let l = side.first_line();
        let n = side.fifo[l].len();
        let mut v = (side.csr & !(0x1F << 16)) | side.sticky;
        let (frf, fwf) = if tx {
            (n <= side.watermark(), n == 0)
        } else {
            (n > side.watermark(), n >= FIFO)
        };
        if frf {
            v |= XCSR_FRF;
        }
        if fwf {
            v |= XCSR_FWF;
        }
        v
    }

    fn side_irq(side: &Side, tx: bool) -> bool {
        let v = Self::csr_view(side, tx);
        (v & XCSR_FRIE != 0 && v & XCSR_FRF != 0)
            || (v & XCSR_FWIE != 0 && v & XCSR_FWF != 0)
            || (v & XCSR_FEIE != 0 && v & XCSR_FEF != 0)
            || (v & XCSR_SEIE != 0 && v & XCSR_SEF != 0)
            || (v & XCSR_WSIE != 0 && v & XCSR_WSF != 0)
    }

    pub fn read_reg(&self, off: u32) -> u32 {
        self.sync();
        let mut guard = self.inner.borrow_mut();
        let i = &mut *guard;
        match off & !3 {
            0x000 => 0x0300_0000,
            0x004 => 0x0005_0504,
            0x008 => Self::csr_view(&i.tx, true),
            o @ 0x00C..=0x01C => i.tx.cr[((o - 0x0C) / 4) as usize],
            0x020..=0x02C => 0, // TDR is write-only
            o @ 0x040..=0x04C => {
                let l = ((o - 0x40) / 4) as usize;
                let n = i.tx.fifo[l].len() as u32;
                (n & 0x3F) << 16 // WFP - RFP = count (RFP kept at 0)
            }
            0x060 => i.tx.mr,
            0x088 => Self::csr_view(&i.rx, false),
            o @ 0x08C..=0x09C => i.rx.cr[((o - 0x8C) / 4) as usize],
            o @ 0x0A0..=0x0AC => {
                let l = ((o - 0xA0) / 4) as usize;
                i.rx.fifo[l].pop_front().unwrap_or(0)
            }
            o @ 0x0C0..=0x0CC => {
                let l = ((o - 0xC0) / 4) as usize;
                ((i.rx.fifo[l].len() as u32) & 0x3F) << 16
            }
            0x0E0 => i.rx.mr,
            _ => 0,
        }
    }

    pub fn write_reg(&mut self, off: u32, value: u32, mask: u32) {
        self.sync();
        let now = self.time.now();
        let mut guard = self.inner.borrow_mut();
        let i = &mut *guard;
        let v = value & mask;
        let write_csr = |side: &mut Side, old_en: bool| {
            let new = (side.csr & !mask) | v;
            side.sticky &= !(v & (XCSR_FEF | XCSR_SEF | XCSR_WSF));
            if new & XCSR_FR != 0 {
                side.fifo.iter_mut().for_each(|q| q.clear());
            }
            if new & XCSR_SR != 0 {
                side.fifo.iter_mut().for_each(|q| q.clear());
                side.sticky = 0;
            }
            side.csr = new & !(XCSR_FR | 0x1F << 16);
            if !old_en && side.enabled() {
                side.t0 = now;
                side.frames = 0;
            }
        };
        match off & !3 {
            0x008 => {
                let en = i.tx.enabled();
                write_csr(&mut i.tx, en);
            }
            o @ 0x00C..=0x01C => {
                let k = ((o - 0x0C) / 4) as usize;
                i.tx.cr[k] = (i.tx.cr[k] & !mask) | v;
            }
            o @ 0x020..=0x02C => {
                let l = ((o - 0x20) / 4) as usize;
                if i.tx.fifo[l].len() < FIFO {
                    i.tx.fifo[l].push_back(v);
                } else {
                    i.tx.sticky |= XCSR_FEF;
                }
            }
            0x060 => i.tx.mr = (i.tx.mr & !mask) | v,
            0x088 => {
                let en = i.rx.enabled();
                write_csr(&mut i.rx, en);
            }
            o @ 0x08C..=0x09C => {
                let k = ((o - 0x8C) / 4) as usize;
                i.rx.cr[k] = (i.rx.cr[k] & !mask) | v;
            }
            0x0E0 => i.rx.mr = (i.rx.mr & !mask) | v,
            _ => {}
        }
    }

    fn irq(&self) -> bool {
        self.sync();
        let i = self.inner.borrow();
        let tx = Self::side_irq(&i.tx, true);
        self.tx_level.set(tx);
        if self.tx_irq.is_some() {
            Self::side_irq(&i.rx, false)
        } else {
            tx || Self::side_irq(&i.rx, false)
        }
    }

    fn next_frame_cycle(&self) -> Option<u64> {
        let i = self.inner.borrow();
        let mut best: Option<u64> = None;
        for (own, other) in [(&i.tx, &i.rx), (&i.rx, &i.tx)] {
            if !own.enabled() {
                continue;
            }
            if let Some(fc) = self.frame_cycles(own, other) {
                let t = own.t0 + (own.frames + 1) * fc;
                best = Some(best.map_or(t, |b| b.min(t)));
            }
        }
        best
    }

    fn refresh_irq(&self) {
        self.time.set_level(self.irq());
    }
}

impl Peripheral for ImxrtSai {
    fn legacy_tick_active(&self) -> bool {
        let i = self.inner.borrow();
        i.tx.enabled() || i.rx.enabled() || self.time.level()
    }
    fn legacy_tick_dynamic(&self) -> bool {
        true
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        let v = byte_of(self.read_reg(offset as u32), offset);
        self.refresh_irq();
        Ok(v)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = (offset & 3) * 8;
        self.write_reg(offset as u32, (value as u32) << shift, 0xFF << shift);
        self.refresh_irq();
        Ok(())
    }
    /// One FIFO access per halfword: eDMA moves 16-bit samples with 16-bit
    /// transfers, and the byte default would push or pop one word per byte.
    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        let v = self.read_reg(offset as u32) >> ((offset & 2) * 8);
        self.refresh_irq();
        Ok(v as u16)
    }
    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        let shift = (offset & 2) * 8;
        self.write_reg(offset as u32, (value as u32) << shift, 0xFFFF << shift);
        self.refresh_irq();
        Ok(())
    }
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = self.read_reg(offset as u32);
        self.refresh_irq();
        Ok(v)
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset as u32, value, u32::MAX);
        self.refresh_irq();
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        let off = offset as u32 & !3;
        if (0xA0..0xB0).contains(&off) {
            let i = self.inner.borrow();
            let l = ((off - 0xA0) / 4) as usize;
            return Some(byte_of(i.rx.fifo[l].front().copied().unwrap_or(0), offset));
        }
        Some(byte_of(self.read_reg(off), offset))
    }
    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.time.advance(cycles);
        self.sync();
        let tx_was = self.tx_level.get();
        self.refresh_irq();
        let mut r = super::wake_hint(self.time.now(), self.next_frame_cycle());
        if let Some(line) = self.tx_irq {
            // Separate transmitter line: pulse it on the rising edge.
            if self.tx_level.get() && !tx_was {
                r.explicit_irqs = Some(vec![line]);
            }
        }
        r
    }
    fn irq_line_level(&self) -> Option<bool> {
        Some(self.time.level())
    }
    /// Line 0: transmit (FRDE & FRF, FWDE & FWF); line 1: receive.
    fn dma_request_active(&self, line: u8) -> bool {
        self.sync();
        let i = self.inner.borrow();
        let (side, tx) = match line {
            0 => (&i.tx, true),
            1 => (&i.rx, false),
            _ => return false,
        };
        let v = Self::csr_view(side, tx);
        (v & XCSR_FRDE != 0 && v & XCSR_FRF != 0) || (v & XCSR_FWDE != 0 && v & XCSR_FWF != 0)
    }
    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.time.attach_clock(clock);
    }
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.time.attach_cpu_hz(hz);
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn snapshot(&self) -> serde_json::Value {
        let i = self.inner.borrow();
        serde_json::json!({
            "peripheral": "imxrt_sai",
            "tcsr": Self::csr_view(&i.tx, true),
            "rcsr": Self::csr_view(&i.rx, false),
            "tx_frames": i.tx.frames,
            "rx_frames": i.rx.frames,
            "tx_words": i.tx_words,
            "tx_peak": i.tx_peak,
            "rx_amp": i.rx_amp,
            "rx_hz": i.rx_hz,
        })
    }

    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        f(self)
    }

    fn logs(&self) -> Vec<crate::peripheral_log::PeripheralLog> {
        let i = self.inner.borrow();
        let tail = tail_peak(&i.captured[0]);
        vec![
            crate::peripheral_log::PeripheralLog::new(
                "rx",
                vec![format!(
                    "words {} amp {} hz {}",
                    i.rx_words, i.rx_amp, i.rx_hz as u64
                )],
            ),
            crate::peripheral_log::PeripheralLog::new(
                "tx",
                vec![format!(
                    "words {} peak {} tail {}",
                    i.tx_words, i.tx_peak, tail
                )],
            ),
        ]
    }
}

/// The `rx_wav` sample at the current position (linear interpolation).
fn wav_sample(w: &RxWav) -> u32 {
    let k = w.pos.floor() as usize;
    let Some(&a) = w.samples.get(k) else {
        return 0;
    };
    let b = w.samples.get(k + 1).copied().unwrap_or(0.0);
    let t = (w.pos - k as f64) as f32;
    let v = ((a + (b - a) * t) as f64 * w.gain * 32767.0)
        .round()
        .clamp(-32768.0, 32767.0);
    (v as i16) as u16 as u32
}

impl Drop for ImxrtSai {
    fn drop(&mut self) {
        self.flush_tx_wav();
    }
}

fn tail_peak(words: &VecDeque<u32>) -> u32 {
    words.iter().rev().take(TAIL_WORDS).fold(0u32, |peak, w| {
        let sample = (*w as u16) as i16;
        peak.max(u32::from(sample.unsigned_abs()))
    })
}

/// Line 0 slot 0 while `rx_amp` is set. Every other slot on that line is 0,
/// so a stereo frame is one sample then a silent slot. Other lines keep the
/// queued source.
fn rx_sample(i: &mut Inner, cpu_hz: u64, frame_cycles: u64, line: usize, slot: u32) -> u32 {
    if line == 0 && i.rx_amp != 0 {
        if slot != 0 {
            return 0;
        }
        let sample = if i.rx_hz <= 0.0 || frame_cycles == 0 || cpu_hz == 0 {
            i.rx_amp
        } else {
            let sr = cpu_hz as f64 / frame_cycles as f64;
            let s = (i.rx_amp as f64 * (i.rx_phase * std::f64::consts::TAU).sin()).round();
            i.rx_phase = (i.rx_phase + i.rx_hz / sr).rem_euclid(1.0);
            s.clamp(-32767.0, 32767.0) as i32
        };
        return (sample as i16) as u16 as u32;
    }
    i.rx_source
        .get_mut(line)
        .and_then(|q| q.pop_front())
        .unwrap_or(0)
}

impl SimInput for ImxrtSai {
    fn input_channels(&self) -> &[InputChannel] {
        RX_INPUTS
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), SimInputError> {
        self.require_channel(key, value)?;
        let i = self.inner.get_mut();
        match key {
            "rx_amp" => i.rx_amp = value.round() as i32,
            _ => i.rx_hz = value,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CycleClock;

    #[test]
    fn tx_frames_consume_the_fifo_at_the_programmed_rate() {
        let mut s = ImxrtSai::default();
        let c = CycleClock::default();
        s.attach_cycle_clock(c.clone());
        // I2S master: BCLK = MCLK/4 (DIV=1), 2 x 32-bit words per frame.
        s.write_reg(0x0C, 16, u32::MAX); // TFW
        s.write_reg(0x10, (1 << 24) | 1, u32::MAX); // BCD, DIV=1
        s.write_reg(0x14, 1 << 16, u32::MAX); // TCE line 0
        s.write_reg(0x18, 1 << 16, u32::MAX); // FRSZ = 1 (2 words)
        s.write_reg(0x1C, (31 << 24) | (31 << 16), u32::MAX);
        for k in 0..4 {
            s.write_reg(0x20, k, u32::MAX);
        }
        s.write_reg(0x08, XCSR_EN, u32::MAX);
        let fc = 64 * 600_000_000 / (DEFAULT_MCLK_HZ / 4);
        c.publish(fc);
        s.read_reg(0x08);
        assert_eq!(s.frames().0, 1);
        assert_eq!(s.tx_capture(0).1, vec![0, 1]);
        c.publish(3 * fc);
        s.read_reg(0x08);
        assert_ne!(
            s.read_reg(0x08) & XCSR_FEF,
            0,
            "underrun after the FIFO drained"
        );
    }

    fn enable_rx(s: &mut ImxrtSai) {
        s.write_reg(0x8C, 16, u32::MAX);
        s.write_reg(0x90, (1 << 24) | 1, u32::MAX);
        s.write_reg(0x94, 1 << 16, u32::MAX);
        s.write_reg(0x98, 1 << 16, u32::MAX);
        s.write_reg(0x9C, (31 << 24) | (31 << 16), u32::MAX);
        s.write_reg(0x88, XCSR_EN, u32::MAX);
    }

    #[test]
    fn rx_hold_is_the_first_slot_and_the_logs_name_the_level() {
        let mut s = ImxrtSai::default();
        let c = CycleClock::default();
        s.attach_cycle_clock(c.clone());
        s.set_input("rx_amp", 1000.0).unwrap();
        s.set_input("rx_hz", 0.0).unwrap();
        enable_rx(&mut s);
        let fc = 64 * 600_000_000 / (DEFAULT_MCLK_HZ / 4);
        c.publish(fc);
        assert_eq!(s.read_reg(0xA0), 1000);
        assert_eq!(s.read_reg(0xA0), 0);
        let logs = s.logs();
        assert_eq!(logs.len(), 2);
        assert!(logs[0].entries.iter().any(|e| e.text.contains("amp 1000")));
        assert!(logs[1].entries.iter().any(|e| e.text.contains("words ")));
    }

    #[test]
    fn rx_tone_changes_from_sample_to_sample() {
        let mut s = ImxrtSai::default();
        let c = CycleClock::default();
        s.attach_cycle_clock(c.clone());
        s.set_input("rx_amp", 8000.0).unwrap();
        s.set_input("rx_hz", 1000.0).unwrap();
        enable_rx(&mut s);
        let fc = 64 * 600_000_000 / (DEFAULT_MCLK_HZ / 4);
        c.publish(fc * 8);
        let mut words = Vec::new();
        for _ in 0..8 {
            words.push(s.read_reg(0xA0) as u16 as i16);
            let silent = s.read_reg(0xA0);
            assert_eq!(silent, 0);
        }
        assert!(words.iter().any(|w| *w != 0));
        assert!(words.windows(2).any(|p| p[0] != p[1]));
    }

    fn enable_tx(s: &mut ImxrtSai) {
        s.write_reg(0x0C, 16, u32::MAX);
        s.write_reg(0x10, (1 << 24) | 1, u32::MAX);
        s.write_reg(0x14, 1 << 16, u32::MAX);
        s.write_reg(0x18, 1 << 16, u32::MAX);
        s.write_reg(0x1C, (31 << 24) | (31 << 16), u32::MAX);
        s.write_reg(0x08, XCSR_EN, u32::MAX);
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("labwired-sai-{}-{name}", std::process::id()))
    }

    #[test]
    fn halfword_fifo_access_moves_one_word_per_sample() {
        // eDMA moves 16-bit samples with 16-bit transfers: one TDR write
        // and one RDR read per sample, not one per byte.
        let mut s = ImxrtSai::default();
        let c = CycleClock::default();
        s.attach_cycle_clock(c.clone());
        s.write_u16(0x20, 0x1234).unwrap();
        s.write_u16(0x20, 0xFF00).unwrap();
        enable_tx(&mut s);
        let fc = 64 * 600_000_000 / (DEFAULT_MCLK_HZ / 4);
        c.publish(fc);
        s.read_reg(0x08);
        assert_eq!(s.tx_capture(0).1, vec![0x1234, 0xFF00]);

        s.set_input("rx_amp", -300.0).unwrap();
        enable_rx(&mut s);
        c.publish(2 * fc);
        assert_eq!(s.read_u16(0xA0).unwrap() as i16, -300);
        assert_eq!(
            s.read_u16(0xA0).unwrap(),
            0,
            "slot 1, not the sample's high byte"
        );
    }

    #[test]
    fn rx_wav_plays_into_slot_0_and_tx_wav_records_both_slots() {
        let input = tmp("in.wav");
        let output = tmp("out.wav");
        // 44.1 kHz mono 16-bit: 0.5, -0.25.
        let mut f = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        f.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 1] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.extend_from_slice(&44_100u32.to_le_bytes());
        f.extend_from_slice(&88_200u32.to_le_bytes());
        for v in [2u16, 16] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.extend_from_slice(b"data");
        f.extend_from_slice(&4u32.to_le_bytes());
        for v in [16384i16, -8192] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&input, f).unwrap();

        let mut s = ImxrtSai::default()
            .with_rx_wav(&input, 1.0)
            .unwrap()
            .with_tx_wav(&output);
        let c = CycleClock::default();
        s.attach_cycle_clock(c.clone());
        // 16-bit stereo I2S at 44.1 kHz: BCLK = MCLK/8 (DIV=3), 2 x 16 bits.
        for base in [0x0Cu32, 0x8C] {
            s.write_reg(base + 4, (1 << 24) | 3, u32::MAX);
            s.write_reg(base + 8, 1 << 16, u32::MAX);
            s.write_reg(base + 12, 1 << 16, u32::MAX);
            s.write_reg(base + 16, (15 << 24) | (15 << 16), u32::MAX);
        }
        for w in [100u16, 200, 300, 400] {
            s.write_u16(0x20, w).unwrap();
        }
        s.write_reg(0x08, XCSR_EN, u32::MAX);
        s.write_reg(0x88, XCSR_EN, u32::MAX);
        let fc = 32 * 600_000_000 / (DEFAULT_MCLK_HZ / 8);
        c.publish(2 * fc);
        s.read_reg(0x08);
        let rx: Vec<i16> = (0..4).map(|_| s.read_u16(0xA0).unwrap() as i16).collect();
        assert_eq!(rx, vec![16384, 0, -8192, 0]);

        s.flush_tx_wav();
        let b = std::fs::read(&output).unwrap();
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 44_100);
        let pcm: Vec<i16> = b[44..]
            .chunks_exact(2)
            .map(|p| i16::from_le_bytes([p[0], p[1]]))
            .collect();
        assert_eq!(pcm, vec![100, 200, 300, 400]);
        let _ = std::fs::remove_file(input);
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn rx_wav_rejects_a_file_that_is_not_wav() {
        let p = tmp("bad.wav");
        std::fs::write(&p, b"not a wave").unwrap();
        assert!(ImxrtSai::default().with_rx_wav(&p, 1.0).is_err());
        let _ = std::fs::remove_file(p);
    }
}
