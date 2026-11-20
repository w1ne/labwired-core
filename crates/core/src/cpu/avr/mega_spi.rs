// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! SPI byte exchange with attached device models, and the ATmega640/1280/
//! 1281/2560/2561 SPI block timing (Microchip DS2549, "SPI - Serial
//! Peripheral Interface").
//!
//! | Register | Addr | Bits |
//! |----------|------|------|
//! | `SPCR` | 0x4C | SPIE SPE DORD MSTR CPOL CPHA SPR1 SPR0 |
//! | `SPSR` | 0x4D | SPIF WCOL (read-only), SPI2X |
//! | `SPDR` | 0x4E | data |
//!
//! Pins: SS = PB0, SCK = PB1, MOSI = PB2, MISO = PB3. Devices are the same
//! byte-level models the 328P reaches: those the CPU owns and those left on
//! the bus parking SPI controller, each selected by its own chip-select pin.
//!
//! Master mode: writing SPDR shifts the byte out (LSB first with DORD) and
//! the device answers at once; SPIF is set after eight SCK periods, SCK =
//! fosc / {4, 16, 64, 128} for SPR1:0, doubled by SPI2X (Table 21-5). The
//! received byte appears in SPDR then. Writing SPDR during a transfer sets
//! WCOL and is ignored. SPIF and WCOL clear by reading SPSR with the flag set
//! and then accessing SPDR, and SPIF also on entry to the SPI vector (328P
//! number 18, 2560 vector 25). SPSR bits 7..1 are read-only.
//!
//! "If SS is configured as an input and is driven low while MSTR is set, MSTR
//! will be cleared, and SPIF in SPSR will become set." The pad is checked at
//! instruction boundaries.
//!
//! The four SPI modes (CPOL/CPHA) are stored in SPCR; the device models are
//! byte-level and exchange the same byte in every mode, so the SCK idle level
//! and sampling edge are not drawn on the pads. Slave mode keeps the byte
//! written to SPDR, but no external master model clocks it.

use super::{Avr, AVR_IO_MIRROR_BASE};
use crate::{Bus, SimResult};
use std::cell::Cell;

pub(super) const ADDR_SPCR: u16 = 0x4C;
pub(super) const ADDR_SPSR: u16 = 0x4D;
pub(super) const ADDR_SPDR: u16 = 0x4E;

const SPIE: u8 = 1 << 7;
const SPE: u8 = 1 << 6;
const DORD: u8 = 1 << 5;
const MSTR: u8 = 1 << 4;
const SPIF: u8 = 1 << 7;
const WCOL: u8 = 1 << 6;
const SPI2X: u8 = 1 << 0;

/// `SPI_STC_vect` in this core's 328P numbering.
pub(super) const VEC_SPI_STC: u32 = 18;
/// `PINB`; SS is PB0.
const PINB: u16 = 0x23;
const SS: u8 = 1 << 0;

#[derive(Debug, Default)]
pub(super) struct MegaSpi {
    busy: bool,
    done_at: u64,
    /// Byte the running transfer receives.
    rx: u8,
    /// SPDR as firmware reads it: the last byte received.
    rx_buf: u8,
    /// SPIF/WCOL seen set by the last SPSR read.
    armed: Cell<u8>,
    /// Flags an SPDR access since cleared, applied at the next boundary.
    cleared: Cell<u8>,
}

pub(super) fn handles(addr: u16) -> bool {
    (ADDR_SPCR..=ADDR_SPDR).contains(&addr)
}

impl Avr {
    /// Clock one byte through every selected device: those the CPU owns, then
    /// the kits on the bus parking SPI controller. Returns MISO.
    pub(super) fn spi_exchange(&mut self, bus: &mut dyn Bus, mosi: u8) -> u8 {
        self.sync_spi_cs(bus);
        // Sample D/C for every device that declares a `dc_source` (same
        // semantics as SystemBus::maybe_latch_dc), generic for any SPI panel
        // with a DC line.
        let mut miso = 0u8;
        let sources: Vec<(usize, u64, u8)> = self
            .spi_devices
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.dc_source().map(|(a, b)| (i, a, b)))
            .collect();
        for (i, addr, bit) in sources {
            let level = bus
                .read_u32(addr)
                .map(|v| (v >> bit) & 1 != 0)
                .unwrap_or(false);
            self.spi_devices[i].set_dc_level(level);
        }
        // Clock selected devices and continuous shift registers. An inactive
        // LOAD may clock DOUT, but it does not select that chain as the
        // source of the controller's MISO byte.
        for dev in &mut self.spi_devices {
            if !dev.is_selected() && !dev.clocks_when_deselected() {
                continue;
            }
            let resp = dev.transfer(mosi);
            if dev.is_selected() && resp != 0 {
                miso = resp;
            }
        }
        // Kits left on the parking SPI controller: latch D/C and clock them
        // so inspect / display_artifact still see the panel.
        let bus_miso = bus.clock_parking_spi(mosi);
        if bus_miso != 0 {
            miso = bus_miso;
        }
        miso
    }

    /// CPU clocks per SCK period (Table 21-5).
    fn mega_spi_div(&self) -> u64 {
        let base = [4u64, 16, 64, 128][usize::from(self.spcr & 3)];
        if self.spsr & SPI2X != 0 {
            base / 2
        } else {
            base
        }
    }

    fn mega_spi_sync_irq(&mut self) {
        let bit = 1u64 << VEC_SPI_STC;
        if self.spcr & SPIE != 0 && self.spsr & SPIF != 0 {
            self.pending_irq |= bit;
        } else {
            self.pending_irq &= !bit;
        }
    }

    /// Master mode with SS an input: a low SS is a mode fault.
    #[inline]
    fn mega_spi_ss_watched(&self) -> bool {
        self.spcr & (SPE | MSTR) == SPE | MSTR && self.io[usize::from(PINB + 1 - 0x20)] & SS == 0
    }

    /// Something the boundary poll must look at.
    #[inline]
    pub(super) fn mega_spi_active(&self) -> bool {
        self.mspi.busy || self.mspi.cleared.get() != 0 || self.mega_spi_ss_watched()
    }

    /// The SPSR-then-SPDR clear, applied now.
    fn mega_spi_apply_clear(&mut self) {
        let c = self.mspi.cleared.take();
        if c != 0 {
            self.spsr &= !c;
            self.mega_spi_sync_irq();
        }
    }

    /// Instruction-boundary work: transfer completion and the SS mode fault.
    pub(super) fn mega_spi_poll(&mut self, bus: &dyn Bus) {
        self.mega_spi_apply_clear();
        if self.mspi.busy && self.cycles >= self.mspi.done_at {
            self.mspi.busy = false;
            self.mspi.rx_buf = self.mspi.rx;
            self.spdr = self.mspi.rx;
            self.spsr |= SPIF;
            self.mega_spi_sync_irq();
        }
        if self.mega_spi_ss_watched() {
            let external = bus
                .read_u8(AVR_IO_MIRROR_BASE + u64::from(PINB))
                .unwrap_or(SS);
            if external & SS == 0 {
                self.spcr &= !MSTR;
                self.spsr |= SPIF;
                self.mega_spi_sync_irq();
            }
        }
    }

    /// Cycles until the running transfer completes, for the idle skip.
    pub(super) fn mega_spi_wake_cycles(&self) -> Option<u64> {
        if self.mspi.busy {
            Some(self.mspi.done_at.saturating_sub(self.cycles).max(1))
        } else if self.mega_spi_ss_watched() {
            // The SS pad may fall on any cycle.
            Some(1)
        } else {
            None
        }
    }

    pub(super) fn mega_spi_vector_entered(&mut self) {
        self.spsr &= !SPIF;
    }

    /// An SPDR access after an SPSR read that saw SPIF/WCOL clears them.
    fn mega_spi_spdr_accessed(&self) {
        let armed = self.mspi.armed.take();
        self.mspi.cleared.set(self.mspi.cleared.get() | armed);
    }

    pub(super) fn mega_spi_read(&self, addr: u16) -> u8 {
        match addr {
            ADDR_SPSR => {
                let v = self.spsr & !self.mspi.cleared.get();
                self.mspi
                    .armed
                    .set(self.mspi.armed.get() | (v & (SPIF | WCOL)));
                v
            }
            ADDR_SPDR => {
                self.mega_spi_spdr_accessed();
                self.mspi.rx_buf
            }
            _ => self.spcr,
        }
    }

    pub(super) fn mega_spi_write(
        &mut self,
        addr: u16,
        value: u8,
        bus: &mut dyn Bus,
    ) -> SimResult<()> {
        match addr {
            ADDR_SPCR => {
                self.spcr = value;
                self.mega_spi_sync_irq();
            }
            ADDR_SPSR => {
                self.spsr = (self.spsr & (SPIF | WCOL)) | (value & SPI2X);
            }
            _ => {
                self.mega_spi_spdr_accessed();
                self.mega_spi_apply_clear();
                if self.mspi.busy {
                    self.spsr |= WCOL;
                    return Ok(());
                }
                if self.spcr & (SPE | MSTR) != SPE | MSTR {
                    // Slave (or disabled): the byte waits for a master.
                    return Ok(());
                }
                let lsb_first = self.spcr & DORD != 0;
                let wire = if lsb_first {
                    value.reverse_bits()
                } else {
                    value
                };
                let miso = self.spi_exchange(bus, wire);
                self.mspi.rx = if lsb_first { miso.reverse_bits() } else { miso };
                self.mspi.busy = true;
                self.mspi.done_at = self.cycles + 8 * self.mega_spi_div();
            }
        }
        Ok(())
    }

    pub(super) fn mega_spi_reset(&mut self) {
        self.mspi = MegaSpi::default();
    }
}
