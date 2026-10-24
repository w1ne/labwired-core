// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A bit-level SPI wire engine for a controller whose pads sit on a world
//! `gpio_net`, shared by register models that do not carry their own (today
//! the nRF52 SPIM and SPIS, `spi_nrf_net` and `nrf52::spis`).
//!
//! The engine owns the wire and nothing else: SCK, MOSI and MISO levels, the
//! clock phase and polarity, bit order, and the shift registers. Every
//! register-level decision (which word goes out next, what a received word
//! means, chip select policy) belongs to the host model:
//!
//! * **Master.** [`SpiBitBus::m_begin`] starts a frame; the engine clocks it
//!   at [`SpiBitBus::half`] cycles per SCK half period from
//!   [`SpiBitBus::deadline`], drives SCK and MOSI push-pull and samples the
//!   level the net delivers at the MISO pad at each sampling edge (the
//!   leading edge at CPHA = 0, the trailing one at CPHA = 1). At the last
//!   edge it reports [`SpiEvent::MDone`]; a host that starts the next frame
//!   in the same cycle keeps the clock running back to back.
//! * **Slave.** It has no clock of its own. The host tells it when it is
//!   selected ([`SpiBitBus::s_select`]) and hands it every SCK level the net
//!   delivers ([`SpiBitBus::s_sck`]): it shifts MOSI in on the sampling edge
//!   and puts the next bit on MISO on the shift edge. Each time a new word is
//!   needed it asks the host through a closure, so the word can come from
//!   wherever the register model keeps it. A full word is reported as
//!   [`SpiEvent::SWord`]. MISO is driven only while selected.
//!
//! Timed steps run at their exact cycle; slave edges run when the net
//! delivers them. Nothing is polled.

use crate::peripherals::pad_lines::{LineDrive, PadLines};

/// Which [`PadLines`] index carries each SPI signal for the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpiLines {
    pub sck: usize,
    pub mosi: usize,
    pub miso: usize,
}

/// Frame shape.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpiShape {
    /// SCK idle level.
    pub cpol: bool,
    /// Sample on the trailing (true) or leading (false) edge.
    pub cpha: bool,
    pub lsb_first: bool,
    /// Frame size in bits (1..=16).
    pub bits: u8,
}

impl SpiShape {
    /// Bit `idx` (in shift order) of `word`.
    fn bit(&self, word: u16, idx: u8) -> bool {
        if idx >= self.bits {
            return false;
        }
        let pos = if self.lsb_first {
            idx
        } else {
            self.bits - 1 - idx
        };
        (word >> pos) & 1 != 0
    }

    /// Put `level` at bit `idx` (in shift order) of `word`.
    fn set_bit(&self, word: &mut u16, idx: u8, level: bool) {
        if idx >= self.bits {
            return;
        }
        let pos = if self.lsb_first {
            idx
        } else {
            self.bits - 1 - idx
        };
        if level {
            *word |= 1 << pos;
        } else {
            *word &= !(1 << pos);
        }
    }
}

/// What the wire did, for the host to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpiEvent {
    /// The master's frame is complete; the word sampled on MISO.
    MDone(u16),
    /// A slave frame is complete: the word sampled on MOSI.
    SWord(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Lead,
    Trail,
}

#[derive(Debug, Clone, Copy)]
struct MFrame {
    word: u16,
    rx: u16,
    idx: u8,
    step: Step,
}

/// The engine state. See the module docs.
#[derive(Debug, Default)]
pub(crate) struct SpiBitBus {
    pub(crate) shape: SpiShape,
    /// SCK half period in engine cycles (master).
    pub(crate) half: u64,
    m: Option<MFrame>,
    deadline: Option<u64>,
    // slave
    s_selected: bool,
    s_sck: bool,
    s_bits: u8,
    s_rx: u16,
    s_tx: Option<u16>,
    events: Vec<SpiEvent>,
}

impl SpiBitBus {
    /// Events since the last call, oldest first.
    pub(crate) fn take_events(&mut self) -> Vec<SpiEvent> {
        std::mem::take(&mut self.events)
    }

    /// The cycle of the next timed step, if one is due.
    pub(crate) fn deadline(&self) -> Option<u64> {
        self.deadline
    }

    /// `true` while a master frame is on the wire.
    pub(crate) fn master_busy(&self) -> bool {
        self.m.is_some()
    }

    /// `true` while selected as a slave.
    pub(crate) fn selected(&self) -> bool {
        self.s_selected
    }

    /// Forget any frame in flight (the controller was disabled). The host
    /// re-states the line drives.
    pub(crate) fn reset(&mut self) {
        let (shape, half) = (self.shape, self.half);
        *self = Self {
            shape,
            half,
            ..Self::default()
        };
    }

    // ── master ──────────────────────────────────────────────────────────

    /// Drive SCK at its idle level and MOSI push-pull: a master between
    /// frames.
    pub(crate) fn m_idle_drive(&self, lines: SpiLines, pads: &PadLines) {
        if self.m.is_none() {
            pads.drive_line(lines.sck, LineDrive::PushPull, self.shape.cpol);
        }
        pads.set_mode(lines.mosi, LineDrive::PushPull);
        pads.set_mode(lines.miso, LineDrive::Input);
    }

    /// Start clocking `word` out now. At CPHA = 0 its first bit goes on MOSI
    /// at once, half a period before the first edge.
    pub(crate) fn m_begin(&mut self, word: u16, now: u64, lines: SpiLines, pads: &PadLines) {
        pads.drive_line(lines.sck, LineDrive::PushPull, self.shape.cpol);
        if !self.shape.cpha {
            pads.drive_line(lines.mosi, LineDrive::PushPull, self.shape.bit(word, 0));
        }
        self.m = Some(MFrame {
            word,
            rx: 0,
            idx: 0,
            step: Step::Lead,
        });
        self.deadline = Some(now + self.half.max(1));
    }

    /// Run every timed step due by `now`, each at its own cycle. The host
    /// handles the events after each call; a frame ends the run of steps.
    pub(crate) fn step(&mut self, now: u64, lines: SpiLines, pads: &PadLines) {
        while let Some(due) = self.deadline {
            if due > now {
                break;
            }
            self.deadline = None;
            pads.set_stamp(Some(due));
            self.step_at(due, lines, pads);
        }
    }

    fn step_at(&mut self, now: u64, lines: SpiLines, pads: &PadLines) {
        let Some(mut f) = self.m else {
            return;
        };
        let shape = self.shape;
        let sample = |f: &mut MFrame| {
            let bit = pads.wire_level(lines.miso);
            shape.set_bit(&mut f.rx, f.idx, bit);
        };
        match f.step {
            Step::Lead => {
                pads.drive_line(lines.sck, LineDrive::PushPull, !shape.cpol);
                if shape.cpha {
                    pads.drive_line(lines.mosi, LineDrive::PushPull, shape.bit(f.word, f.idx));
                } else {
                    sample(&mut f);
                }
                f.step = Step::Trail;
            }
            Step::Trail => {
                pads.drive_line(lines.sck, LineDrive::PushPull, shape.cpol);
                if shape.cpha {
                    sample(&mut f);
                } else if f.idx + 1 < shape.bits {
                    pads.drive_line(
                        lines.mosi,
                        LineDrive::PushPull,
                        shape.bit(f.word, f.idx + 1),
                    );
                }
                f.idx += 1;
                if f.idx >= shape.bits {
                    self.m = None;
                    self.events.push(SpiEvent::MDone(f.rx));
                    return;
                }
                f.step = Step::Lead;
            }
        }
        self.m = Some(f);
        self.deadline = Some(now + self.half.max(1));
    }

    // ── slave ───────────────────────────────────────────────────────────

    /// Select or deselect the slave. Selected, it drives MISO (the first bit
    /// of the word `next_tx` gives at CPHA = 0); deselected, it lets go of
    /// MISO and drops a partly shifted frame.
    pub(crate) fn s_select(
        &mut self,
        selected: bool,
        lines: SpiLines,
        pads: &PadLines,
        next_tx: &mut dyn FnMut() -> u16,
    ) {
        if selected == self.s_selected {
            return;
        }
        self.s_selected = selected;
        self.s_bits = 0;
        self.s_rx = 0;
        self.s_tx = None;
        if selected {
            self.s_sck = pads.input(lines.sck).unwrap_or(self.shape.cpol);
            if self.shape.cpha {
                pads.set_mode(lines.miso, LineDrive::PushPull);
            } else {
                let word = next_tx();
                self.s_tx = Some(word);
                pads.drive_line(lines.miso, LineDrive::PushPull, self.shape.bit(word, 0));
            }
        } else {
            pads.set_mode(lines.miso, LineDrive::Input);
        }
    }

    /// The net delivered a new SCK level at the slave's pad.
    pub(crate) fn s_sck(
        &mut self,
        level: bool,
        lines: SpiLines,
        pads: &PadLines,
        next_tx: &mut dyn FnMut() -> u16,
    ) {
        if !self.s_selected || level == self.s_sck {
            return;
        }
        self.s_sck = level;
        let shape = self.shape;
        let leading = level != shape.cpol;
        if leading != shape.cpha {
            // Sampling edge.
            if self.s_tx.is_none() {
                self.s_tx = Some(next_tx());
            }
            let bit = pads.wire_level(lines.mosi);
            shape.set_bit(&mut self.s_rx, self.s_bits, bit);
            self.s_bits += 1;
            if self.s_bits >= shape.bits {
                let word = self.s_rx;
                self.s_bits = 0;
                self.s_rx = 0;
                self.s_tx = None;
                self.events.push(SpiEvent::SWord(word));
            }
        } else {
            // Shift edge: the next bit (the next word's first at a frame
            // boundary) goes out.
            let word = match self.s_tx {
                Some(w) => w,
                None => {
                    let w = next_tx();
                    self.s_tx = Some(w);
                    w
                }
            };
            pads.drive_line(
                lines.miso,
                LineDrive::PushPull,
                shape.bit(word, self.s_bits),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &["SCK", "MOSI", "MISO"];
    const L: SpiLines = SpiLines {
        sck: 0,
        mosi: 1,
        miso: 2,
    };

    /// A master and a slave on zero-latency wires.
    fn exchange(shape: SpiShape, m_words: &[u16], s_words: &[u16]) -> (Vec<u16>, Vec<u16>) {
        let pm = PadLines::new(NAMES, &[shape.cpol, false, false]);
        let ps = PadLines::new(NAMES, &[shape.cpol, false, false]);
        let mut m = SpiBitBus {
            shape,
            half: 4,
            ..Default::default()
        };
        let mut s = SpiBitBus {
            shape,
            ..Default::default()
        };
        let mut tx = s_words.iter().copied();
        let mut next = move || tx.next().unwrap_or(0);
        m.m_idle_drive(L, &pm);
        ps.set_input(L.sck, shape.cpol);
        s.s_select(true, L, &ps, &mut next);
        let mut sck_seen = shape.cpol;
        let mut got_m = Vec::new();
        let mut got_s = Vec::new();
        let mut queue = m_words.iter().copied();
        m.m_begin(queue.next().unwrap(), 0, L, &pm);
        loop {
            // Deliver the wires, MOSI before the clock edge it was set up for.
            ps.set_input(L.mosi, pm.level(L.mosi));
            let sck = pm.level(L.sck);
            if sck != sck_seen {
                sck_seen = sck;
                ps.set_input(L.sck, sck);
                s.s_sck(sck, L, &ps, &mut next);
            }
            for e in s.take_events() {
                if let SpiEvent::SWord(w) = e {
                    got_s.push(w);
                }
            }
            pm.set_input(L.miso, ps.level(L.miso));
            let Some(due) = m.deadline() else {
                break;
            };
            m.step(due, L, &pm);
            for e in m.take_events() {
                if let SpiEvent::MDone(w) = e {
                    got_m.push(w);
                    if let Some(next_word) = queue.next() {
                        m.m_begin(next_word, due, L, &pm);
                    }
                }
            }
        }
        (got_m, got_s)
    }

    #[test]
    fn every_mode_and_bit_order_crosses_whole_words() {
        for mode in 0..8u8 {
            let shape = SpiShape {
                cpol: mode & 1 != 0,
                cpha: mode & 2 != 0,
                lsb_first: mode & 4 != 0,
                bits: 8,
            };
            let (m, s) = exchange(shape, &[0xA5, 0x3C, 0x01], &[0x81, 0x82, 0xF0]);
            assert_eq!(s, vec![0xA5, 0x3C, 0x01], "slave saw, {shape:?}");
            assert_eq!(m, vec![0x81, 0x82, 0xF0], "master saw, {shape:?}");
        }
    }

    #[test]
    fn a_deselected_slave_lets_go_of_miso() {
        let p = PadLines::new(NAMES, &[false, false, false]);
        let mut s = SpiBitBus {
            shape: SpiShape {
                bits: 8,
                ..Default::default()
            },
            ..Default::default()
        };
        s.s_select(true, L, &p, &mut || 0x80);
        assert!(p.drives(L.miso) && p.level(L.miso), "MSB of 0x80 first");
        s.s_select(false, L, &p, &mut || 0);
        assert!(!p.drives(L.miso));
    }
}
