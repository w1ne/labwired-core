// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF52 SPIS (EasyDMA SPI slave) on a world `gpio_net`: a bit-level
//! slave between chips, on the shared wire engine
//! ([`crate::peripherals::spi_bitbus`]).
//!
//! When a pad the SPIS's `PSEL.SCK`, `PSEL.MOSI`, `PSEL.MISO` or `PSEL.CSN`
//! names is on a net (and `ENABLE = 2`), the slave reacts to the CSN, SCK and
//! MOSI levels the net delivers (nRF52840 PS v1.11 §6.26):
//!
//! * CSN low starts a transaction. With the semaphore free (the CPU did
//!   `TASKS_RELEASE`) the slave takes it (`SEMSTAT = 2`) and the transaction
//!   is granted: MISO shifts out the TX buffer (`TXD.PTR`/`MAXCNT`, read by
//!   EasyDMA at the release), then `ORC` past its end (`STATUS.OVERREAD`);
//!   MOSI bytes fill the RX buffer up to `RXD.MAXCNT`, then are dropped
//!   (`STATUS.OVERFLOW`). With the CPU holding the semaphore the slave sends
//!   `DEF` and ignores what it receives.
//! * CSN high ends it: for a granted transaction the RX bytes reach RAM at
//!   `RXD.PTR`, `RXD.AMOUNT` / `TXD.AMOUNT` count what crossed, and
//!   `EVENTS_ENDRX` and `EVENTS_END` fire. The semaphore then goes to the CPU
//!   (`EVENTS_ACQUIRED`) if it asked for it or `SHORTS.END_ACQUIRE` is set,
//!   else it is free for the next transaction.
//! * `CONFIG` gives the frame shape (CPOL, CPHA, bit order). MISO is driven
//!   only while CSN is low; SCK, MOSI and CSN are inputs.

use super::{
    Nrf52Spis, ENABLE_MASK, ENABLE_SPIS, LINE_CSN, LINE_MISO, LINE_MOSI, LINE_SCK, MAXCNT_MASK,
    SEM_FREE, SEM_SPIS, STATUS_OVERFLOW, STATUS_OVERREAD,
};
use crate::peripherals::pad_lines::{LineDrive, PadLines};
use crate::peripherals::spi_bitbus::{SpiBitBus, SpiEvent, SpiLines, SpiShape};

const LINES: SpiLines = SpiLines {
    sck: LINE_SCK,
    mosi: LINE_MOSI,
    miso: LINE_MISO,
};

/// The net state, kept beside the register file.
#[derive(Debug, Default)]
pub(super) struct SpisNet {
    bus: SpiBitBus,
    /// The TX buffer as EasyDMA read it when the CPU released the semaphore.
    tx: Vec<u8>,
    /// EasyDMA owes a read of the TX buffer.
    prefetch: bool,
    /// The transaction in progress is granted (the slave holds the
    /// semaphore).
    granted: bool,
    /// Words handed to the shift register in this transaction.
    handed: usize,
    /// Frames completed in this transaction.
    frames: usize,
    rx: Vec<u8>,
    /// A granted transaction ended: EasyDMA owes the RX write and the events.
    finish: bool,
    armed: Vec<u64>,
    irq_armed: bool,
}

impl Nrf52Spis {
    fn net_pads(&self) -> Option<std::sync::Arc<PadLines>> {
        self.lines.clone()
    }

    /// `true` when the slave is enabled on pads that sit on a world
    /// `gpio_net`.
    pub(super) fn net_mode(&self) -> bool {
        self.lines.is_some()
            && (self.claim_sck.on_net()
                || self.claim_mosi.on_net()
                || self.claim_miso.on_net()
                || self.claim_csn.on_net())
    }

    fn net_now(&self) -> u64 {
        self.clock.as_ref().map_or(0, crate::CycleClock::now)
    }

    fn net_shape(&self) -> SpiShape {
        SpiShape {
            lsb_first: self.config & 1 != 0,
            cpha: self.config & 2 != 0,
            cpol: self.config & 4 != 0,
            bits: 8,
        }
    }

    /// The CPU released the semaphore: EasyDMA reads the TX buffer.
    pub(super) fn net_released(&mut self) {
        if self.lines.is_some() {
            self.net.prefetch = true;
        }
    }

    pub(super) fn net_needs_bus(&self) -> bool {
        self.net.prefetch || self.net.finish
    }

    /// EasyDMA: the TX buffer after a release, the RX buffer after a granted
    /// transaction.
    pub(super) fn net_dma(&mut self, bus: &mut dyn crate::Bus) {
        if std::mem::take(&mut self.net.prefetch) {
            let ptr = u64::from(self.txd_ptr);
            let n = (self.txd_maxcnt & MAXCNT_MASK) as usize;
            self.net.tx = crate::with_device_write_bypass(bus, |bus| {
                (0..n)
                    .map(|i| bus.read_u8(ptr + i as u64).unwrap_or(0))
                    .collect()
            });
        }
        if std::mem::take(&mut self.net.finish) {
            let ptr = u64::from(self.rxd_ptr);
            crate::with_device_write_bypass(bus, |bus| {
                for (i, &b) in self.net.rx.iter().enumerate() {
                    let _ = bus.write_u8(ptr + i as u64, b);
                }
            });
            self.rxd_amount = self.net.rx.len() as u32;
            let txd_max = (self.txd_maxcnt & MAXCNT_MASK) as usize;
            self.txd_amount = self.net.frames.min(txd_max) as u32;
            self.events_endrx = 1;
            self.events_end = 1;
            self.transaction_ended();
        }
    }

    /// After `ENABLE`, `PSEL` or `CONFIG`: re-state the lines; a slave that
    /// left its net forgets the transaction.
    pub(super) fn net_after_pins(&mut self) {
        let Some(pads) = self.net_pads() else {
            return;
        };
        if self.net_mode() {
            if !self.net.bus.selected() {
                self.net.bus.shape = self.net_shape();
            }
            // Selected by a CSN already low when the slave came up.
            if pads.input(LINE_CSN) == Some(false) {
                let now = self.net_now();
                self.net_edge(LINE_CSN, false, now);
            }
        } else if self.net.bus.selected() || self.enable & ENABLE_MASK != ENABLE_SPIS {
            self.net.bus.reset();
            for line in [LINE_SCK, LINE_MOSI, LINE_MISO, LINE_CSN] {
                pads.set_mode(line, LineDrive::Input);
            }
        }
    }

    /// The net delivered a new level on one of this slave's lines.
    pub(super) fn net_edge(&mut self, line: usize, level: bool, now: u64) -> bool {
        if !self.net_mode() {
            return false;
        }
        let Some(pads) = self.net_pads() else {
            return false;
        };
        pads.set_stamp(Some(now));
        let orc = (self.orc & 0xFF) as u8;
        let def = (self.def & 0xFF) as u8;
        match line {
            LINE_CSN => {
                let select = !level;
                if select && !self.net.bus.selected() {
                    // A new transaction: granted when the semaphore is free.
                    self.net.granted = self.semstat & 0x3 == SEM_FREE;
                    if self.net.granted {
                        self.semstat = SEM_SPIS;
                    }
                    self.net.handed = 0;
                    self.net.frames = 0;
                    self.net.rx.clear();
                    self.net.bus.shape = self.net_shape();
                } else if !select && self.net.bus.selected() && self.net.granted {
                    self.net.granted = false;
                    self.net.finish = true;
                }
                let net = &mut self.net;
                let (tx, granted, handed) = (&net.tx, net.granted, &mut net.handed);
                let mut next = || next_word(tx, granted, handed, orc, def);
                net.bus.s_select(select, LINES, &pads, &mut next);
            }
            LINE_SCK => {
                let net = &mut self.net;
                let (tx, granted, handed) = (&net.tx, net.granted, &mut net.handed);
                let mut next = || next_word(tx, granted, handed, orc, def);
                net.bus.s_sck(level, LINES, &pads, &mut next);
                for event in net.bus.take_events() {
                    let SpiEvent::SWord(word) = event else {
                        continue;
                    };
                    if !net.granted {
                        continue;
                    }
                    if net.frames >= net.tx.len() {
                        self.status |= STATUS_OVERREAD;
                    }
                    net.frames += 1;
                    if net.rx.len() < (self.rxd_maxcnt & MAXCNT_MASK) as usize {
                        net.rx.push(word as u8);
                    } else {
                        self.status |= STATUS_OVERFLOW;
                    }
                }
            }
            _ => {}
        }
        pads.set_stamp(None);
        self.net_needs_bus() || self.irq_level()
    }

    /// Scheduler path: EasyDMA work runs on the next cycle; the interrupt is
    /// a level, re-raised while it holds.
    pub(super) fn net_take_events(&mut self) -> Vec<(u64, u32)> {
        let now = self.net_now();
        if self.net_needs_bus() {
            let due = now + 1;
            if !self.net.armed.contains(&due) {
                self.net.armed.push(due);
                return vec![(0, 0)];
            }
            return Vec::new();
        }
        if self.irq_level() && !self.net.irq_armed {
            self.net.irq_armed = true;
            return vec![(0, 0)];
        }
        Vec::new()
    }

    pub(super) fn net_on_sched(&mut self, bus: &mut dyn crate::Bus) -> crate::sched::EventResult {
        let now = self.net_now();
        self.net.armed.retain(|&t| t > now);
        self.net.irq_armed = false;
        self.net_dma(bus);
        let irq = self.irq_level();
        if irq {
            self.net.irq_armed = true;
        }
        crate::sched::EventResult {
            raise_own_irq: irq,
            reschedule_delay: irq.then_some(1),
            ..Default::default()
        }
    }
}

/// The next word a slave shifts out: the TX buffer, then `ORC`, for a granted
/// transaction; `DEF` otherwise.
fn next_word(tx: &[u8], granted: bool, handed: &mut usize, orc: u8, def: u8) -> u16 {
    let i = *handed;
    *handed += 1;
    if !granted {
        return u16::from(def);
    }
    u16::from(tx.get(i).copied().unwrap_or(orc))
}
