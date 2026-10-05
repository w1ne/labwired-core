// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The STM32 classic/FIFO SPI on a world `gpio_net`: bit-level master and
//! slave between two chips.
//!
//! Off a net the engine in `spi.rs` is self-contained: the master clocks a
//! frame on its own pads and takes MISO from a byte-level device model. When a
//! pad that can carry one of this controller's lines is put on a net
//! ([`PadLines::on_net`]), the wire belongs to several chips and this module
//! takes over what the outside decides:
//!
//! * **Drive.** Every line says what its output stage does
//!   ([`LineDrive`]): a master drives SCK and MOSI push-pull and treats MISO
//!   as an input; a slave treats SCK, MOSI and NSS as inputs and drives MISO
//!   only while selected. A disabled controller (SPE=0) drives nothing. The
//!   net resolves the wire from that, so two masters fighting show up as
//!   `GPIO_NET_CONTENTION` rather than being hidden.
//! * **Master receive.** The master samples the level the net delivers to its
//!   MISO pad at its own sampling edge (the boundary into the second half of
//!   each bit period, which is the leading edge at CPHA=0 and the trailing one
//!   at CPHA=1) and that is what lands in DR.
//! * **Slave.** A slave has no clock of its own. It reacts to the SCK, MOSI
//!   and NSS levels the net delivers to its pads
//!   ([`crate::Peripheral::wire_input_edge`]), which arrive at the exact cycle
//!   the wire carries them there: event-driven, nothing is polled per cycle.
//!   On its sampling edge it shifts MOSI in; on its shift edge it puts the
//!   next bit of its transmit word on MISO. A full frame lands in DR with
//!   RXNE (and the SPI interrupt when CR2.RXNEIE is set); a frame arriving
//!   while RXNE is still set sets OVR and is lost, as on silicon.
//!
//! Selection: with CR1.SSM=1 the slave is selected while CR1.SSI=0; with
//! SSM=0 it follows the level on its NSS pad (low selects). An NSS pad the
//! net has never driven reads as not selected. A master with CR2.SSOE=1 and
//! SSM=0 drives NSS low while SPE=1.
//!
//! Timing limit, the physical one: MISO answers a clock edge only after the
//! edge has crossed the wire to the slave and the answer has crossed back, so
//! two net latencies must fit in half an SCK period. Slow the SCK (CR1.BR) or
//! shorten `latency_ns` when it does not.

use super::{Spi, SpiRegs, SpiSignal};
use crate::peripherals::pad_lines::{LineDrive, PadLines};

const CR1_CPOL: u16 = 1 << 1;
const CR1_MSTR: u16 = 1 << 2;
const CR1_SPE: u16 = 1 << 6;
const CR1_SSI: u16 = 1 << 8;
const CR1_SSM: u16 = 1 << 9;
const CR2_SSOE: u16 = 1 << 2;
const CR2_RXNEIE: u16 = 1 << 6;
const CR2_TXEIE: u16 = 1 << 7;
const SR_RXNE: u16 = 1 << 0;
const SR_TXE: u16 = 1 << 1;
const SR_OVR: u16 = 1 << 6;
const SR_BSY: u16 = 1 << 7;

const SCK: usize = SpiSignal::Sck as usize;
const MOSI: usize = SpiSignal::Mosi as usize;
const MISO: usize = SpiSignal::Miso as usize;
const NSS: usize = SpiSignal::Nss as usize;

/// A slave's shift state between net edges.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct NetSlave {
    selected: bool,
    /// The SCK level last seen at the pad.
    sck: bool,
    /// Bits sampled so far in the frame on the wire.
    bits: u8,
    rx: u16,
    /// The word being shifted out on MISO, once loaded.
    tx: Option<u16>,
}

impl Spi {
    fn net_lines(&self) -> Option<&PadLines> {
        self.lines.as_ref().map(|l| &**l.pad_lines())
    }

    /// `true` when this classic/FIFO STM32 SPI shares its wire with other
    /// chips through a world `gpio_net`.
    pub(super) fn net_mode(&self) -> bool {
        matches!(self.regs, SpiRegs::Stm32(_)) && self.net_lines().is_some_and(PadLines::any_on_net)
    }

    fn net_cr(&self) -> (u16, u16) {
        match &self.regs {
            SpiRegs::Stm32(r) => (r.cr1, r.cr2),
            _ => (0, 0),
        }
    }

    /// `true` when a net-mode slave is enabled (SPE=1, MSTR=0).
    pub(super) fn net_slave_active(&self) -> bool {
        let (cr1, _) = self.net_cr();
        self.net_mode() && cr1 & CR1_SPE != 0 && cr1 & CR1_MSTR == 0
    }

    /// Re-state every line's drive from CR1/CR2 and the slave's selection.
    /// Called after every CR1/CR2 write and selection change; costs one
    /// branch off a net.
    pub(super) fn net_apply_drive(&mut self) {
        if !self.net_mode() {
            return;
        }
        let (cr1, cr2) = self.net_cr();
        let spe = cr1 & CR1_SPE != 0;
        let master = cr1 & CR1_MSTR != 0;
        let selected = self.net_slave.selected;
        let lines = self.lines.clone().expect("net mode has lines");
        let pads = lines.pad_lines();
        let cpol = cr1 & CR1_CPOL != 0;
        if spe && master {
            // SCK rests at CPOL between frames; a frame in flight owns it.
            let sck = if self.frame.is_some() {
                pads.level(SCK)
            } else {
                cpol
            };
            pads.drive_line(SCK, LineDrive::PushPull, sck);
            pads.set_mode(MOSI, LineDrive::PushPull);
            pads.set_mode(MISO, LineDrive::Input);
            if cr2 & CR2_SSOE != 0 && cr1 & CR1_SSM == 0 {
                pads.drive_line(NSS, LineDrive::PushPull, false);
            } else {
                pads.set_mode(NSS, LineDrive::Input);
            }
        } else {
            pads.set_mode(SCK, LineDrive::Input);
            pads.set_mode(MOSI, LineDrive::Input);
            pads.set_mode(NSS, LineDrive::Input);
            if spe && selected {
                pads.set_mode(MISO, LineDrive::PushPull);
            } else {
                pads.set_mode(MISO, LineDrive::Input);
            }
        }
        if spe && !master {
            self.net_slave_update_select();
        }
    }

    /// The master's MISO sample for the bit now on the wire: the level the
    /// net holds at the MISO pad, stored at the bit's place in the frame.
    pub(super) fn net_master_sample(&mut self) {
        let Some(f) = &self.frame else {
            return;
        };
        let Some(pads) = self.net_lines() else {
            return;
        };
        let bit = pads.wire_level(MISO);
        let pos = if f.t.lsb_first {
            f.bit_idx
        } else {
            f.t.bits - 1 - f.bit_idx
        };
        if bit {
            self.net_rx |= 1 << pos;
        } else {
            self.net_rx &= !(1 << pos);
        }
    }

    /// Frame width and shape from the live registers.
    fn net_frame_shape(&self) -> (u8, bool, bool, bool) {
        let t = self.stm32_frame_timing();
        (t.bits, t.cpol, t.cpha, t.lsb_first)
    }

    fn net_bit_of(word: u16, index: u8, bits: u8, lsb_first: bool) -> bool {
        if index >= bits {
            return false;
        }
        let pos = if lsb_first { index } else { bits - 1 - index };
        (word >> pos) & 1 != 0
    }

    /// Load the slave's next transmit word from the TX buffer, if it has
    /// none yet. An empty buffer shifts out zeros.
    fn net_slave_load_tx(&mut self) {
        if self.net_slave.tx.is_some() {
            return;
        }
        if let Some(word) = self.tx_queue.pop_front() {
            self.net_slave.tx = Some(word);
            let mut irq = false;
            if let SpiRegs::Stm32(r) = &mut self.regs {
                r.sr |= SR_TXE;
                irq = r.cr2 & CR2_TXEIE != 0;
            }
            if irq {
                self.net_irq_pending = true;
            }
        }
    }

    /// Put bit `index` of the transmit word on MISO (while selected).
    fn net_slave_present(&mut self, index: u8) {
        if !self.net_slave.selected {
            return;
        }
        let (bits, _, _, lsb) = self.net_frame_shape();
        let word = self.net_slave.tx.unwrap_or(0);
        let level = Self::net_bit_of(word, index, bits, lsb);
        if let Some(pads) = self.net_lines() {
            pads.drive_line(MISO, LineDrive::PushPull, level);
        }
    }

    /// Recompute selection from SSM/SSI or the NSS pad and act on a change.
    fn net_slave_update_select(&mut self) {
        let (cr1, _) = self.net_cr();
        let spe = cr1 & CR1_SPE != 0;
        let selected = spe
            && cr1 & CR1_MSTR == 0
            && if cr1 & CR1_SSM != 0 {
                cr1 & CR1_SSI == 0
            } else {
                self.net_lines().and_then(|p| p.input(NSS)) == Some(false)
            };
        if selected == self.net_slave.selected {
            return;
        }
        self.net_slave.selected = selected;
        self.net_slave.bits = 0;
        self.net_slave.rx = 0;
        let (_, cpol, cpha, _) = self.net_frame_shape();
        if selected {
            self.net_slave.sck = self.net_lines().and_then(|p| p.input(SCK)).unwrap_or(cpol);
            self.net_slave_load_tx();
            if let Some(pads) = self.net_lines() {
                pads.set_mode(MISO, LineDrive::PushPull);
            }
            // CPHA=0: the first bit is on MISO before the first edge.
            if !cpha {
                self.net_slave_present(0);
            }
        } else {
            if let Some(pads) = self.net_lines() {
                pads.set_mode(MISO, LineDrive::Input);
            }
            if let SpiRegs::Stm32(r) = &mut self.regs {
                r.sr &= !SR_BSY;
            }
        }
    }

    /// A frame finished on the slave: the word lands in DR with RXNE, or is
    /// lost to OVR while the previous one is still unread.
    fn net_slave_complete(&mut self) {
        let rx = self.net_slave.rx;
        self.net_slave.rx = 0;
        self.net_slave.bits = 0;
        self.net_slave.tx = None;
        self.net_frames += 1;
        let rxne_set = self.stm32_rxne.get();
        let mut irq = false;
        if let SpiRegs::Stm32(r) = &mut self.regs {
            r.sr &= !SR_BSY;
            if rxne_set {
                r.sr |= SR_OVR;
            } else {
                r.dr = rx;
                r.sr |= SR_RXNE;
                self.stm32_rxne.set(true);
                irq = r.cr2 & CR2_RXNEIE != 0;
            }
        }
        if irq {
            self.net_irq_pending = true;
        }
        self.net_slave_load_tx();
    }

    /// The outside moved the level on one of this slave's pads.
    pub(super) fn net_slave_edge(&mut self, line: usize, level: bool) -> bool {
        if !self.net_slave_active() {
            return false;
        }
        match line {
            NSS => self.net_slave_update_select(),
            SCK if self.net_slave.selected => {
                if level == self.net_slave.sck {
                    return false;
                }
                self.net_slave.sck = level;
                let (bits, cpol, cpha, lsb) = self.net_frame_shape();
                let leading = level != cpol;
                // CPHA=0 samples on the leading edge, CPHA=1 on the trailing.
                let sample = leading != cpha;
                if sample {
                    let bit = self.net_lines().is_some_and(|p| p.wire_level(MOSI));
                    let idx = self.net_slave.bits;
                    let pos = if lsb { idx } else { bits - 1 - idx };
                    if bit {
                        self.net_slave.rx |= 1 << pos;
                    }
                    self.net_slave.bits += 1;
                    if let SpiRegs::Stm32(r) = &mut self.regs {
                        r.sr |= SR_BSY;
                    }
                    if self.net_slave.bits >= bits {
                        self.net_slave_complete();
                    }
                } else {
                    // The shift edge: the next bit goes out. A CPHA=1 frame
                    // takes its word at its first edge if none was loaded.
                    if self.net_slave.bits == 0 {
                        self.net_slave_load_tx();
                    }
                    self.net_slave_present(self.net_slave.bits);
                }
            }
            _ => return false,
        }
        self.net_irq_pending
    }

    /// A DR write on a net-mode slave: the word waits in the TX buffer (one
    /// deep, a second write overwrites it) until a frame takes it. A selected
    /// slave between frames takes it at once, so a CPHA=0 first bit is on
    /// MISO before the master's first edge.
    pub(super) fn net_slave_write_dr(&mut self, value: u16) {
        self.tx_queue.clear();
        self.tx_queue.push_back(value);
        if let SpiRegs::Stm32(r) = &mut self.regs {
            r.sr &= !SR_TXE;
        }
        if self.net_slave.selected && self.net_slave.bits == 0 && self.net_slave.tx.is_none() {
            self.net_slave_load_tx();
            // CPHA=0 with SCK at rest: the first bit goes out now. Mid-pulse
            // (the trailing edge of the previous frame still to come) it
            // waits for that shift edge, as a shift register would.
            let (_, cpol, cpha, _) = self.net_frame_shape();
            if !cpha && self.net_slave.sck == cpol {
                self.net_slave_present(0);
            }
        }
    }

    /// RXNEIE on a net master's completed frame (the net exchange is the only
    /// place a master receives a real MISO word, so it is the only place this
    /// interrupt is raised from the master side).
    pub(super) fn net_master_rx_irq(&self) -> bool {
        let (_, cr2) = self.net_cr();
        cr2 & CR2_RXNEIE != 0
    }

    /// Frames a net-mode slave has received (diagnostics and tests).
    pub fn net_slave_frames(&self) -> u64 {
        self.net_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn slave(cr1: u16) -> Spi {
        let mut spi = Spi::new();
        let lines = spi.line_levels_arc();
        for line in 0..4 {
            lines.pad_lines().mark_on_net(line);
        }
        spi.write_u16(0x04, CR2_RXNEIE).unwrap();
        spi.write_u16(0x00, cr1).unwrap();
        spi
    }

    fn clock_byte(spi: &mut Spi, byte: u8, cpol: bool) -> Vec<bool> {
        // Mode 0/2 master: data set while SCK idles, leading edge samples.
        let mut miso = Vec::new();
        for i in (0..8).rev() {
            let bit = (byte >> i) & 1 != 0;
            spi.lines.as_ref().unwrap().pad_lines().set_input(MOSI, bit);
            spi.wire_input_edge(MOSI, bit, 0);
            miso.push(spi.lines.as_ref().unwrap().pad_lines().level(MISO));
            spi.lines
                .as_ref()
                .unwrap()
                .pad_lines()
                .set_input(SCK, !cpol);
            spi.wire_input_edge(SCK, !cpol, 0);
            spi.lines.as_ref().unwrap().pad_lines().set_input(SCK, cpol);
            spi.wire_input_edge(SCK, cpol, 0);
        }
        miso
    }

    fn select(spi: &mut Spi, low: bool) {
        spi.lines.as_ref().unwrap().pad_lines().set_input(NSS, !low);
        spi.wire_input_edge(NSS, !low, 0);
    }

    #[test]
    fn a_slave_shifts_mosi_in_and_its_word_out_on_the_nets_clock() {
        let mut spi = slave(CR1_SPE);
        spi.write_u16(0x0C, 0x3C).unwrap();
        assert!(!spi.lines.as_ref().unwrap().pad_lines().drives(MISO));
        select(&mut spi, true);
        assert!(spi.lines.as_ref().unwrap().pad_lines().drives(MISO));
        let miso = clock_byte(&mut spi, 0xA5, false);
        let answered = miso.iter().fold(0u8, |acc, &b| (acc << 1) | u8::from(b));
        assert_eq!(
            answered, 0x3C,
            "MISO carries the slave's DR word, MSB first"
        );
        assert_eq!(spi.read(0x08).unwrap() & 1, 1, "RXNE");
        assert_eq!(spi.read(0x0C).unwrap(), 0xA5);
        assert!(spi.net_irq_pending, "RXNEIE raises the interrupt");
        select(&mut spi, false);
        assert!(!spi.lines.as_ref().unwrap().pad_lines().drives(MISO));
    }

    #[test]
    fn a_second_frame_before_dr_is_read_sets_ovr_and_is_lost() {
        let mut spi = slave(CR1_SPE);
        select(&mut spi, true);
        clock_byte(&mut spi, 0x11, false);
        clock_byte(&mut spi, 0x22, false);
        assert_ne!(spi.read(0x08).unwrap() & (SR_OVR as u8), 0);
        assert_eq!(spi.read(0x0C).unwrap(), 0x11);
    }

    #[test]
    fn an_unselected_slave_ignores_the_clock() {
        let mut spi = slave(CR1_SPE);
        clock_byte(&mut spi, 0x5A, false);
        assert_eq!(spi.read(0x08).unwrap() & 1, 0);
        assert_eq!(spi.net_slave_frames(), 0);
    }

    #[test]
    fn software_nss_selects_while_ssi_is_low() {
        let mut spi = slave(CR1_SPE | CR1_SSM);
        assert!(spi.net_slave.selected);
        clock_byte(&mut spi, 0xC3, false);
        assert_eq!(spi.read(0x0C).unwrap(), 0xC3);
        spi.write_u16(0x00, CR1_SPE | CR1_SSM | CR1_SSI).unwrap();
        assert!(!spi.net_slave.selected);
    }

    #[test]
    fn a_net_master_drives_clock_and_data_and_listens_on_miso() {
        let mut spi = slave(CR1_SPE | CR1_MSTR);
        let pads = spi.lines.as_ref().unwrap().pad_lines().clone();
        assert!(pads.drives(SCK));
        assert!(pads.drives(MOSI));
        assert!(!pads.drives(MISO));
        assert!(!pads.drives(NSS), "no SSOE: NSS is not driven");
        spi.write_u16(0x00, 0).unwrap();
        assert!(!pads.drives(SCK), "SPE=0 drives nothing");
    }
}
