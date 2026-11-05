// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **RS-485 transceiver gate** — what a MAX485 does between a UART and a
//! shared A/B pair.
//!
//! The transceiver has two halves. The driver puts DI on the bus while DE is
//! high. The receiver puts the bus on RO while /RE is low. Both can be on at
//! once, which is how a master hears its own frame (half-duplex echo). The
//! gate lives inside the [`Uart`](super::uart::Uart) that DI and RO are wired
//! to, because that UART is where the bus's other members already attach: every
//! slave on the pair is one more stream device on that UART, so a multi-drop
//! segment needs no new wiring concept.
//!
//! What the gate decides, byte by byte:
//!
//! * A byte the MCU transmits reaches the bus only while DE is high. With DE
//!   low it still leaves the MCU's TX pin (the console sink sees it), but no
//!   slave hears it.
//! * With DE high and /RE low the MCU receives its own byte back.
//! * A byte a slave puts on the bus reaches the MCU only while /RE is low.
//! * A slave byte while DE is high is a collision with the master's driver, and
//!   so is a byte from two slaves in the same service interval. Both are
//!   reported as contention and neither byte is delivered: two drivers fight on
//!   a real pair and the result is not a valid character.
//!
//! The DE and /RE levels come from level cells the GPIO model keeps equal to
//! the pad (`Peripheral::watch_pad_level`), so the level is the one at the
//! moment of the UART access, not the one at the last scheduler wake-up. That
//! matters here: the model's TX is instantaneous, so firmware drops DE right
//! after `flush()`, and a late sample would see every frame as already over.
//!
//! What the gate does not model: line-level analog behaviour (termination,
//! reflections, biasing), driver slew, a receiver that is enabled during the
//! middle of a character, and the bus turn-around time of the transceiver
//! (a few hundred ns, far below one character).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Where a gate input comes from.
#[derive(Debug, Clone)]
pub enum PinSense {
    /// Tied to a rail on the board.
    Const(bool),
    /// A GPIO pad, kept current by the GPIO model.
    Cell(Arc<AtomicBool>),
    /// The opposite level (an active-low enable read as "enabled").
    Not(Box<PinSense>),
}

impl PinSense {
    #[inline]
    pub(crate) fn level(&self) -> bool {
        match self {
            PinSense::Const(v) => *v,
            PinSense::Cell(c) => c.load(Ordering::Relaxed),
            PinSense::Not(inner) => !inner.level(),
        }
    }
}

/// Who put a run of bytes on the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusSource {
    Master,
    Slave(String),
    Collision,
}

#[derive(Debug, Clone)]
struct BusFrame {
    source: BusSource,
    bytes: Vec<u8>,
    start_cycle: u64,
    last_cycle: u64,
}

/// Frames kept for the log. A long run drops the oldest.
const MAX_FRAMES: usize = 1024;

/// The gate state owned by a UART behind a transceiver.
#[derive(Debug)]
pub struct Rs485Gate {
    id: String,
    de: PinSense,
    re_n: PinSense,
    frames: std::collections::VecDeque<BusFrame>,
    dropped_frames: u64,
    /// Bytes lost to contention.
    pub collisions: u64,
    /// Idle line time, in cycles, that starts a new frame in the log.
    frame_gap_cycles: u64,
}

impl Rs485Gate {
    pub fn new(id: impl Into<String>, de: PinSense, re_n: PinSense) -> Self {
        Self {
            id: id.into(),
            de,
            re_n,
            frames: Default::default(),
            dropped_frames: 0,
            collisions: 0,
            frame_gap_cycles: 0,
        }
    }

    /// The `external_devices` id of the transceiver.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// DE high: this end's driver is on the bus.
    #[inline]
    pub fn driver_enabled(&self) -> bool {
        self.de.level()
    }

    /// /RE low: this end's receiver listens to the bus.
    #[inline]
    pub fn receiver_enabled(&self) -> bool {
        !self.re_n.level()
    }

    /// Cycles of idle line that separate two frames in the log. Set by the UART
    /// from its programmed character time; zero means "split on source only".
    pub fn set_frame_gap_cycles(&mut self, cycles: u64) {
        self.frame_gap_cycles = cycles;
    }

    /// Record bytes on the bus. Consecutive bytes from the same source with no
    /// idle gap join one frame.
    pub fn record(&mut self, source: BusSource, byte: u8, cycle: u64) {
        let join = self.frames.back().is_some_and(|f| {
            f.source == source
                && (self.frame_gap_cycles == 0
                    || cycle.saturating_sub(f.last_cycle) <= self.frame_gap_cycles)
        });
        if join {
            let f = self.frames.back_mut().expect("join implies a last frame");
            f.bytes.push(byte);
            f.last_cycle = cycle;
            return;
        }
        if self.frames.len() >= MAX_FRAMES {
            self.frames.pop_front();
            self.dropped_frames += 1;
        }
        self.frames.push_back(BusFrame {
            source,
            bytes: vec![byte],
            start_cycle: cycle,
            last_cycle: cycle,
        });
    }

    /// The bus log, one line per frame:
    /// `t=1204.512ms master: 01 04 00 00 00 03 B0 0B`.
    ///
    /// `cpu_hz` of zero omits the time stamp.
    pub fn log_lines(&self, cpu_hz: u64) -> Vec<String> {
        self.frames
            .iter()
            .map(|f| {
                let who = match &f.source {
                    BusSource::Master => "master".to_string(),
                    BusSource::Slave(id) => format!("slave {id}"),
                    BusSource::Collision => "collision".to_string(),
                };
                let hex = f
                    .bytes
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                match f.start_cycle.saturating_mul(1_000_000).checked_div(cpu_hz) {
                    Some(us) => format!("t={}.{:03}ms {who}: {hex}", us / 1000, us % 1000),
                    None => format!("{who}: {hex}"),
                }
            })
            .collect()
    }

    /// Frames dropped from the front of the log because it was full.
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames
    }
}
