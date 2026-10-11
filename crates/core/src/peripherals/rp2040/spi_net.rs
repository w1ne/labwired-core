// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The RP2040 PL022 SSP on a world `gpio_net`: a bit-level Motorola-SPI
//! master and slave between chips.
//!
//! Off a net `spi.rs` exchanges each word inside the `SSPDR` write and
//! narrates the waveform afterwards. On a net (a pad that can carry one of
//! this controller's lines is put on one, [`PadLines::on_net`]) the wire
//! belongs to several chips, so the controller runs this engine instead:
//!
//! * **Lines.** SCK, TX (`SSPTXD`), RX (`SSPRXD`) and CSn (`SSPFSSOUT` /
//!   `SSPFSSIN`), routed by IO_BANK0 `FUNCSEL` = SPI. As a master TX is MOSI
//!   and RX is MISO; as a slave (`SSPCR1.MS`) TX is MISO and RX is MOSI.
//! * **Master.** Words written to `SSPDR` queue in the 8-deep TX FIFO; while
//!   `SSE` is set and the FIFO is not empty the engine clocks them out at
//!   `CPSDVSR × (1 + SCR)` cycles per bit (clk_peri taken as the core clock,
//!   as the narrator does), SCK idling at `SPO`, data changed and sampled on
//!   the edges `SPH` gives. CSn goes low for a frame. With `SPH = 0` it
//!   goes high for one SCK period between back-to-back frames; with
//!   `SPH = 1` it stays low until the FIFO is empty (PL022 TRM, Motorola
//!   SPI format). The RX pad is sampled at each sampling edge and the word
//!   lands in the 8-deep RX FIFO (a full FIFO drops it).
//! * **Slave.** It has no clock of its own: it reacts to the CSn, SCK and RX
//!   levels the net delivers. CSn low selects it and it drives TX (unless
//!   `SOD`); each word goes out from the TX FIFO and each received word
//!   lands in the RX FIFO. A frame that starts with the TX FIFO empty sends
//!   zeros and is counted ([`Rp2040Spi::net_underruns`]); the TRM does not
//!   give the line level for that case.
//!
//! Status (`SSPSR`) follows the FIFOs and the shift engine (TFE, TNF, RNE,
//! RFF, BSY). Timed steps run at their exact cycle from a scheduler event;
//! a slave runs when the net delivers an edge. Not modelled on a net: the
//! interrupt registers (`SSPIMSC`/`SSPRIS`/`SSPMIS`/`SSPICR`), DMA, the TI
//! and National Microwire frame formats, and loopback (`LBM`).

use super::{
    Rp2040Spi, CR0_SPH, CR0_SPO, FIFO_DEPTH, LINE_CSN, LINE_MOSI, LINE_RX, LINE_SCK, SR_BSY,
    SR_RFF, SR_RNE, SR_TFE, SR_TNF,
};
use crate::peripherals::pad_lines::{LineDrive, PadLines};
use std::collections::VecDeque;

const CR1_MS: u32 = 1 << 2;
const CR1_SOD: u32 = 1 << 3;

/// Scheduler token for the net engine (the narration chain uses its own
/// sequence numbers, which start at 1 and never reach this).
pub(super) const NET_TOKEN: u32 = 0x5044_0000;

/// Where a master frame is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// The leading SCK edge of bit `idx` is due.
    Lead,
    /// The trailing SCK edge of bit `idx` is due.
    Trail,
    /// Half a period after the last trailing edge: the frame is complete.
    End,
    /// CSn is high between two SPH = 0 frames; the next frame is due.
    Gap,
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    word: u16,
    rx: u16,
    bits: u8,
    idx: u8,
    step: Step,
}

/// The net engine's state, kept beside the word-level model's.
#[derive(Debug, Default)]
pub(super) struct Pl022Net {
    tx: VecDeque<u16>,
    frame: Option<Frame>,
    deadline: Option<u64>,
    armed: Vec<u64>,
    // slave
    selected: bool,
    sck: bool,
    bits: u8,
    rx: u16,
    word: Option<u16>,
    word_was_empty: bool,
    underruns: u64,
}

impl Rp2040Spi {
    fn net_pads(&self) -> Option<std::sync::Arc<PadLines>> {
        self.lines.clone()
    }

    /// `true` when this SSP shares its wire with other chips through a world
    /// `gpio_net`.
    pub(super) fn net_mode(&self) -> bool {
        self.lines.as_deref().is_some_and(PadLines::any_on_net)
    }

    fn net_now(&self) -> u64 {
        self.clock.as_ref().map_or(0, crate::CycleClock::now)
    }

    fn net_slave(&self) -> bool {
        self.cr1 & CR1_MS != 0
    }

    fn net_bits(&self) -> u8 {
        self.framing().bits
    }

    fn net_bit(word: u16, idx: u8, bits: u8) -> bool {
        idx < bits && (word >> (bits - 1 - idx)) & 1 != 0
    }

    /// Frames a net-mode slave sent with its TX FIFO empty (zeros went out).
    pub fn net_underruns(&self) -> u64 {
        self.net.underruns
    }

    /// Re-state every line's drive from SSPCR0/SSPCR1. Called after every
    /// control write; one branch off a net.
    fn net_apply_drive(&mut self) {
        let Some(pads) = self.net_pads() else {
            return;
        };
        let cpol = self.cr0 & CR0_SPO != 0;
        if !self.enabled() {
            for line in [LINE_SCK, LINE_MOSI, LINE_CSN] {
                pads.set_mode(line, LineDrive::Input);
            }
            self.net.frame = None;
            self.net.deadline = None;
            self.net.selected = false;
            return;
        }
        if self.net_slave() {
            pads.set_mode(LINE_SCK, LineDrive::Input);
            pads.set_mode(LINE_CSN, LineDrive::Input);
            self.net_slave_update_select(&pads);
        } else {
            let sck = if self.net.frame.is_some() {
                pads.level(LINE_SCK)
            } else {
                cpol
            };
            pads.drive_line(LINE_SCK, LineDrive::PushPull, sck);
            pads.set_mode(LINE_MOSI, LineDrive::PushPull);
            let cs = self.net.frame.is_none_or(|f| f.step == Step::Gap);
            pads.drive_line(LINE_CSN, LineDrive::PushPull, cs);
            if self.net.frame.is_none() {
                self.net_master_kick();
            }
        }
    }

    /// SSPSR in net mode.
    pub(super) fn net_status(&self) -> u32 {
        let mut sr = 0;
        if self.net.tx.is_empty() {
            sr |= SR_TFE;
        }
        if self.net.tx.len() < FIFO_DEPTH {
            sr |= SR_TNF;
        }
        let rx = self.rx_fifo.borrow();
        if !rx.is_empty() {
            sr |= SR_RNE;
        }
        if rx.len() >= FIFO_DEPTH {
            sr |= SR_RFF;
        }
        let busy = if self.net_slave() {
            self.net.selected && self.net.bits > 0
        } else {
            self.net.frame.is_some() || !self.net.tx.is_empty()
        };
        if busy && self.enabled() {
            sr |= SR_BSY;
        }
        sr
    }

    /// Register writes in net mode. Returns `true` when handled here.
    pub(super) fn net_write(&mut self, offset: u64, value: u32) -> bool {
        let Some(pads) = self.net_pads() else {
            return false;
        };
        pads.set_stamp(Some(self.net_now()));
        let handled = match offset {
            super::SSPCR0 => {
                self.cr0 = value & super::CR0_MASK;
                self.net_apply_drive();
                true
            }
            super::SSPCR1 => {
                self.cr1 = value;
                self.net_apply_drive();
                true
            }
            super::SSPCPSR => {
                self.cpsr = value & super::CPSR_MASK;
                self.net_apply_drive();
                true
            }
            super::SSPDR => {
                if self.net.tx.len() < FIFO_DEPTH {
                    self.net.tx.push_back((value & 0xFFFF) as u16);
                }
                if self.enabled() && !self.net_slave() && self.net.frame.is_none() {
                    self.net_master_kick();
                }
                true
            }
            _ => false,
        };
        pads.set_stamp(None);
        handled
    }

    // ── master ──────────────────────────────────────────────────────────

    fn net_half(&self) -> Option<u64> {
        self.bit_time_cycles().map(|t| (t / 2).max(1))
    }

    /// Start the next frame now if there is a word and a bit rate.
    fn net_master_kick(&mut self) {
        if self.net.tx.is_empty() || self.net_half().is_none() {
            return;
        }
        let now = self.net_now();
        self.net_master_start(now);
    }

    fn net_master_start(&mut self, now: u64) {
        let (Some(pads), Some(half)) = (self.net_pads(), self.net_half()) else {
            return;
        };
        let Some(word) = self.net.tx.pop_front() else {
            self.net.frame = None;
            return;
        };
        let bits = self.net_bits();
        pads.drive_line(LINE_CSN, LineDrive::PushPull, false);
        if self.cr0 & CR0_SPH == 0 {
            // SPH = 0: the first bit is on TX before the first edge.
            pads.drive_line(LINE_MOSI, LineDrive::PushPull, Self::net_bit(word, 0, bits));
        }
        self.net.frame = Some(Frame {
            word,
            rx: 0,
            bits,
            idx: 0,
            step: Step::Lead,
        });
        self.net.deadline = Some(now + half);
    }

    fn net_master_step(&mut self, now: u64, pads: &PadLines) {
        let Some(mut f) = self.net.frame else {
            self.net.deadline = None;
            return;
        };
        let Some(half) = self.net_half() else {
            return;
        };
        let cpol = self.cr0 & CR0_SPO != 0;
        let cpha = self.cr0 & CR0_SPH != 0;
        let sample = |f: &mut Frame| {
            if pads.wire_level(LINE_RX) {
                f.rx |= 1 << (f.bits - 1 - f.idx);
            }
        };
        match f.step {
            Step::Lead => {
                pads.drive_line(LINE_SCK, LineDrive::PushPull, !cpol);
                if cpha {
                    let bit = Self::net_bit(f.word, f.idx, f.bits);
                    pads.drive_line(LINE_MOSI, LineDrive::PushPull, bit);
                } else {
                    sample(&mut f);
                }
                f.step = Step::Trail;
                self.net.deadline = Some(now + half);
            }
            Step::Trail => {
                pads.drive_line(LINE_SCK, LineDrive::PushPull, cpol);
                if cpha {
                    sample(&mut f);
                } else if f.idx + 1 < f.bits {
                    let bit = Self::net_bit(f.word, f.idx + 1, f.bits);
                    pads.drive_line(LINE_MOSI, LineDrive::PushPull, bit);
                }
                f.idx += 1;
                f.step = if f.idx < f.bits {
                    Step::Lead
                } else {
                    Step::End
                };
                self.net.deadline = Some(now + half);
            }
            Step::End => {
                {
                    let mut rx = self.rx_fifo.borrow_mut();
                    if rx.len() < FIFO_DEPTH {
                        rx.push_back(f.rx);
                    }
                }
                if self.net.tx.is_empty() {
                    pads.drive_line(LINE_CSN, LineDrive::PushPull, true);
                    self.net.frame = None;
                    self.net.deadline = None;
                    return;
                }
                if cpha {
                    // SPH = 1: back to back, CSn stays low.
                    self.net.frame = None;
                    self.net_master_start(now);
                    return;
                }
                // SPH = 0: CSn high for one SCK period between frames.
                pads.drive_line(LINE_CSN, LineDrive::PushPull, true);
                f.step = Step::Gap;
                self.net.deadline = Some(now + 2 * half);
            }
            Step::Gap => {
                self.net.frame = None;
                self.net_master_start(now);
                return;
            }
        }
        self.net.frame = Some(f);
    }

    // ── slave ───────────────────────────────────────────────────────────

    fn net_slave_update_select(&mut self, pads: &PadLines) {
        let selected = self.enabled() && self.net_slave() && pads.input(LINE_CSN) == Some(false);
        if selected == self.net.selected {
            return;
        }
        self.net.selected = selected;
        self.net.bits = 0;
        self.net.rx = 0;
        self.net.word = None;
        if selected {
            let cpol = self.cr0 & CR0_SPO != 0;
            self.net.sck = pads.input(LINE_SCK).unwrap_or(cpol);
            if self.cr1 & CR1_SOD == 0 {
                pads.set_mode(LINE_MOSI, LineDrive::PushPull);
            }
            if self.cr0 & CR0_SPH == 0 {
                self.net_slave_load();
                self.net_slave_present(pads);
            }
        } else {
            pads.set_mode(LINE_MOSI, LineDrive::Input);
        }
    }

    fn net_slave_load(&mut self) {
        if self.net.word.is_none() {
            let word = self.net.tx.pop_front();
            self.net.word_was_empty = word.is_none();
            self.net.word = Some(word.unwrap_or(0));
        }
    }

    fn net_slave_present(&mut self, pads: &PadLines) {
        if !self.net.selected || self.cr1 & CR1_SOD != 0 {
            return;
        }
        let bits = self.net_bits();
        let level = Self::net_bit(self.net.word.unwrap_or(0), self.net.bits, bits);
        pads.drive_line(LINE_MOSI, LineDrive::PushPull, level);
    }

    /// The net delivered a new level on one of this controller's lines.
    pub(super) fn net_edge(&mut self, line: usize, level: bool, now: u64) -> bool {
        if !self.net_mode() || !self.enabled() || !self.net_slave() {
            return false;
        }
        let Some(pads) = self.net_pads() else {
            return false;
        };
        pads.set_stamp(Some(now));
        match line {
            LINE_CSN => self.net_slave_update_select(&pads),
            LINE_SCK if self.net.selected && level != self.net.sck => {
                self.net.sck = level;
                let cpol = self.cr0 & CR0_SPO != 0;
                let cpha = self.cr0 & CR0_SPH != 0;
                let bits = self.net_bits();
                let leading = level != cpol;
                if leading != cpha {
                    // Sampling edge.
                    self.net_slave_load();
                    if pads.wire_level(LINE_RX) {
                        self.net.rx |= 1 << (bits - 1 - self.net.bits);
                    }
                    self.net.bits += 1;
                    if self.net.bits >= bits {
                        let mut rx = self.rx_fifo.borrow_mut();
                        if rx.len() < FIFO_DEPTH {
                            rx.push_back(self.net.rx);
                        }
                        drop(rx);
                        if std::mem::take(&mut self.net.word_was_empty) {
                            self.net.underruns += 1;
                        }
                        self.net.bits = 0;
                        self.net.rx = 0;
                        self.net.word = None;
                    }
                } else {
                    // Shift edge: the next bit (the next word's first at a
                    // frame boundary) goes out.
                    self.net_slave_load();
                    self.net_slave_present(&pads);
                }
            }
            _ => {}
        }
        pads.set_stamp(None);
        false
    }

    // ── engine plumbing ─────────────────────────────────────────────────

    fn net_step(&mut self, now: u64) {
        let Some(pads) = self.net_pads() else {
            return;
        };
        while let Some(due) = self.net.deadline {
            if due > now {
                break;
            }
            self.net.deadline = None;
            pads.set_stamp(Some(due));
            self.net_master_step(due, &pads);
        }
        pads.set_stamp(None);
    }

    /// Walk path: one tick.
    pub(super) fn net_tick(&mut self) {
        let now = self.net_now();
        if self.net.deadline.is_some_and(|d| d <= now) {
            self.net_step(now);
        }
    }

    /// Scheduler path: events to arm (delays from now).
    pub(super) fn net_take_events(&mut self) -> Vec<(u64, u32)> {
        let now = self.net_now();
        match self.net.deadline {
            Some(due) => {
                let due = due.max(now + 1);
                if self.net.armed.contains(&due) {
                    return Vec::new();
                }
                self.net.armed.push(due);
                vec![(due - now - 1, NET_TOKEN)]
            }
            None => Vec::new(),
        }
    }

    /// Scheduler path: an armed event fired at `now`.
    pub(super) fn net_on_event(&mut self, now: u64) -> crate::sched::EventResult {
        self.net.armed.retain(|&t| t > now);
        self.net_step(now);
        let mut res = crate::sched::EventResult::default();
        if let Some(due) = self.net.deadline {
            let due = due.max(now + 1);
            if !self.net.armed.contains(&due) {
                self.net.armed.push(due);
                res.reschedule_delay = Some(due - now);
            }
        }
        res
    }
}
