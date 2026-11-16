// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! ATmega640/1280/1281/2560/2561 external interrupts INT0..INT7 and pin-change
//! interrupts PCINT0..23 (Microchip DS2549, "External Interrupts").
//!
//! | Register | Addr | Bits |
//! |----------|------|------|
//! | `PCIFR`  | 0x3B | PCIF2..0, cleared by writing 1 or on vector entry |
//! | `EIFR`   | 0x3C | INTF7..0, cleared by writing 1 or on vector entry |
//! | `EIMSK`  | 0x3D | INT7..0 enables |
//! | `PCICR`  | 0x68 | PCIE2..0 enables |
//! | `EICRA`  | 0x69 | ISC31:30 .. ISC01:00 (INT3..0) |
//! | `EICRB`  | 0x6A | ISC71:70 .. ISC41:40 (INT7..4) |
//! | `PCMSK0..2` | 0x6B..0x6D | per-pin PCINT enables |
//!
//! Pins (DS2549 "Alternate Port Functions"): INT0..3 = PD0..3, INT4..7 =
//! PE4..7; PCINT0..7 = PB0..7, PCINT8 = PE0, PCINT9..15 = PJ0..6,
//! PCINT16..23 = PK0..7. Vectors (Table 14-1): INT0..7 are 2..9, PCINT0..2 are
//! 10..12.
//!
//! Sense control (Tables 15-1 and 15-3): 00 low level, 01 any edge, 10
//! falling, 11 rising. Edges on INT3:0 are detected asynchronously, so they
//! also wake the core from power-down; INT7:4 edges are sampled on clk_I/O and
//! are not seen while it is stopped ("only level interrupt" wakes from
//! power-down through INT7:4, Table 11-1). A level interrupt sets no flag
//! ("INTFn is always cleared when INTn is configured as a level interrupt")
//! and requests for as long as the pad is low. Pin changes are asynchronous.
//!
//! The pads are sampled at instruction boundaries the same way as on the
//! 328P (`ext_int.rs`): an input reads the bus-side port model, an output the
//! core's own latch, so a pad the firmware drives raises its own interrupt.
//!
//! INT0/INT1 and PCINT0..2 use this core's 328P pending numbers (2, 3, 4..6),
//! which [`super::mega2560_vector`] maps onto 2, 3, 10..12. INT2..INT7 have no
//! 328P number: they pend bit `32 + n` of `pending_irq` with `n` the native
//! vector number (4..9), and [`Avr::mega_take_native_irq`] enters them.

use super::{Avr, AVR_IO_MIRROR_BASE, VEC_INT0, VEC_PCINT0};
use crate::Bus;

pub(super) const ADDR_PCIFR: u16 = 0x3B;
pub(super) const ADDR_EIFR: u16 = 0x3C;
pub(super) const ADDR_EIMSK: u16 = 0x3D;
pub(super) const ADDR_PCICR: u16 = 0x68;
pub(super) const ADDR_EICRA: u16 = 0x69;
pub(super) const ADDR_EICRB: u16 = 0x6A;
pub(super) const ADDR_PCMSK0: u16 = 0x6B;
pub(super) const ADDR_PCMSK2: u16 = 0x6D;

/// Native vector numbers of INT2..INT7 (4..9) sit at `pending_irq` bit
/// `NATIVE_BASE + vector` (58..63). Bits 32..57 carry the vectors that only
/// exist on the mega (Timer3/4/5, USART1..3) at their own slot numbers.
pub(super) const NATIVE_BASE: u32 = 54;
/// Native vector number of INT0 (Table 14-1); INTn is `VEC_NATIVE_INT0 + n`.
const VEC_NATIVE_INT0: u32 = 2;
/// `pending_irq` bits this module owns.
const LOW_MASK: u64 = 0b111_1100; // INT0, INT1, PCINT0..2 (328P numbering)
pub(super) const HIGH_MASK: u64 = 0x3F << (NATIVE_BASE + VEC_NATIVE_INT0 + 2); // INT2..7

/// Watched ports, in `MegaExt::last` order.
const PINB: u16 = 0x23;
const PIND: u16 = 0x29;
const PINE: u16 = 0x2C;
const PINJ: u16 = 0x103;
const PINK: u16 = 0x106;
const PORTS: [u16; 5] = [PINB, PIND, PINE, PINJ, PINK];
const B: usize = 0;
const D: usize = 1;
const E: usize = 2;
const J: usize = 3;
const K: usize = 4;

/// State the 328P block has no room for: `EICRB` and the five watched ports.
#[derive(Debug, Default, Clone)]
pub(super) struct MegaExt {
    pub eicrb: u8,
    /// Pad levels of ports B, D, E, J, K at the last boundary sample.
    last: [u8; 5],
}

pub(super) fn handles(addr: u16) -> bool {
    matches!(
        addr,
        ADDR_PCIFR..=ADDR_EIMSK | ADDR_PCICR..=ADDR_EICRB | ADDR_PCMSK0..=ADDR_PCMSK2
    )
}

impl Avr {
    /// The pads of the port whose `PINx` is at `pin`, as `PINx` reads them.
    fn mega_pad_levels(&self, pin: u16, bus: &dyn Bus) -> u8 {
        let ddr = self.io[usize::from(pin + 1 - 0x20)];
        let port = self.port_pad_value(pin + 2, self.io[usize::from(pin + 2 - 0x20)]);
        let external = bus
            .read_u8(AVR_IO_MIRROR_BASE + u64::from(pin))
            .unwrap_or(0);
        (port & ddr) | (external & !ddr)
    }

    /// Sense control of INTn: 0 low level, 1 any edge, 2 falling, 3 rising.
    #[inline]
    fn mega_isc(&self, n: usize) -> u8 {
        let reg = if n < 4 { self.eicra } else { self.mx.eicrb };
        (reg >> (2 * (n % 4))) & 3
    }

    /// Ports that must be sampled at every boundary (bit = index in `PORTS`).
    fn mega_ext_watch(&self) -> u8 {
        let mut w = 0u8;
        if self.pcmsk[0] != 0 {
            w |= 1 << B;
        }
        // An edge flag is kept even while masked, and a level must be tracked
        // while enabled.
        if self.eicra != 0 || self.eimsk & 0x0F != 0 {
            w |= 1 << D;
        }
        if self.mx.eicrb != 0 || self.eimsk & 0xF0 != 0 || self.pcmsk[1] & 1 != 0 {
            w |= 1 << E;
        }
        if self.pcmsk[1] & 0xFE != 0 {
            w |= 1 << J;
        }
        if self.pcmsk[2] != 0 {
            w |= 1 << K;
        }
        w
    }

    /// Re-arm after a configuration write. A port that starts being watched
    /// takes its present level as the reference, so configuring an interrupt
    /// never invents an edge.
    fn mega_ext_config_changed(&mut self, bus: &dyn Bus) {
        let watch = self.mega_ext_watch();
        let fresh = watch & !self.ext_watch;
        for (i, &pin) in PORTS.iter().enumerate() {
            if fresh & (1 << i) != 0 {
                self.mx.last[i] = self.mega_pad_levels(pin, bus);
            }
        }
        self.ext_watch = watch;
        for n in 0..8 {
            if self.mega_isc(n) == 0 {
                self.eifr &= !(1 << n);
            }
        }
        self.mega_sync_ext_pending();
    }

    /// The PCINT8..15 group: PE0 is bit 0, PJ0..6 are bits 1..7.
    #[inline]
    fn pcint1_levels(e: u8, j: u8) -> u8 {
        (e & 1) | (j << 1)
    }

    /// Sample the watched ports at an instruction boundary.
    pub(super) fn mega_sample_ext_pins(&mut self, bus: &dyn Bus) {
        let mut now = self.mx.last;
        for (i, &pin) in PORTS.iter().enumerate() {
            if self.ext_watch & (1 << i) != 0 {
                now[i] = self.mega_pad_levels(pin, bus);
            }
        }
        if now == self.mx.last {
            // A low level keeps requesting after its vector was entered.
            self.mega_sync_ext_pending();
            return;
        }
        let was = self.mx.last;
        self.mx.last = now;
        let groups = [
            (now[B], was[B]),
            (
                Self::pcint1_levels(now[E], now[J]),
                Self::pcint1_levels(was[E], was[J]),
            ),
            (now[K], was[K]),
        ];
        for (g, (n, w)) in groups.iter().enumerate() {
            if (n ^ w) & self.pcmsk[g] != 0 {
                self.pcifr |= 1 << g;
            }
        }
        let io_clock = self.io_clock_running();
        for n in 0..8 {
            let (port, bit) = if n < 4 { (D, n) } else { (E, n) };
            let mask = 1u8 << bit;
            if (now[port] ^ was[port]) & mask == 0 {
                continue;
            }
            // INT7:4 edges are sampled on clk_I/O.
            if n >= 4 && !io_clock {
                continue;
            }
            let rose = now[port] & mask != 0;
            let hit = match self.mega_isc(n) {
                1 => true,
                2 => !rose,
                3 => rose,
                _ => false,
            };
            if hit {
                self.eifr |= 1 << n;
            }
        }
        self.mega_sync_ext_pending();
    }

    /// `true` when a watched pad moved since the last sample.
    pub(super) fn mega_ext_pins_moved(&self, bus: &dyn Bus) -> bool {
        PORTS.iter().enumerate().any(|(i, &pin)| {
            self.ext_watch & (1 << i) != 0 && self.mega_pad_levels(pin, bus) != self.mx.last[i]
        })
    }

    /// Recompute the pending bits of INT0..7 and PCINT0..2.
    fn mega_sync_ext_pending(&mut self) {
        let mut want = 0u64;
        for n in 0..8usize {
            if self.eimsk & (1 << n) == 0 {
                continue;
            }
            let active = if self.mega_isc(n) == 0 {
                let (port, bit) = if n < 4 { (D, n) } else { (E, n) };
                self.mx.last[port] & (1 << bit) == 0
            } else {
                self.eifr & (1 << n) != 0
            };
            if !active {
                continue;
            }
            want |= match n {
                0 | 1 => 1u64 << (VEC_INT0 + n as u32),
                _ => 1u64 << (NATIVE_BASE + VEC_NATIVE_INT0 + n as u32),
            };
        }
        for g in 0..3u32 {
            if self.pcicr & self.pcifr & (1 << g) != 0 {
                want |= 1u64 << (VEC_PCINT0 + g);
            }
        }
        self.pending_irq = (self.pending_irq & !(LOW_MASK | HIGH_MASK)) | want;
    }

    /// Take INT2..INT7 if one is the highest-priority request: only INT0 and
    /// INT1 (vectors 2 and 3) come before them. Returns the native vector
    /// number, with its pending bit and `INTFn` cleared.
    pub(super) fn mega_take_native_irq(&mut self) -> Option<u32> {
        let high = (self.pending_irq & HIGH_MASK) >> NATIVE_BASE;
        if high == 0 || self.pending_irq & (0b11 << VEC_INT0) != 0 {
            return None;
        }
        let vec = high.trailing_zeros();
        self.pending_irq &= !(1u64 << (NATIVE_BASE + vec));
        if let Some(n) = vec.checked_sub(VEC_NATIVE_INT0) {
            if n < 8 {
                self.eifr &= !(1 << n);
            }
        }
        Some(vec)
    }

    pub(super) fn mega_ext_read(&self, addr: u16) -> u8 {
        match addr {
            ADDR_PCIFR => self.pcifr,
            ADDR_EIFR => self.eifr,
            ADDR_EIMSK => self.eimsk,
            ADDR_PCICR => self.pcicr,
            ADDR_EICRA => self.eicra,
            ADDR_EICRB => self.mx.eicrb,
            ADDR_PCMSK0..=ADDR_PCMSK2 => self.pcmsk[usize::from(addr - ADDR_PCMSK0)],
            _ => 0,
        }
    }

    pub(super) fn mega_ext_write(&mut self, addr: u16, value: u8, bus: &dyn Bus) {
        match addr {
            ADDR_PCIFR => {
                self.pcifr &= !(value & 0x07);
                self.mega_sync_ext_pending();
            }
            ADDR_EIFR => {
                self.eifr &= !value;
                self.mega_sync_ext_pending();
            }
            ADDR_EIMSK => {
                self.eimsk = value;
                self.mega_ext_config_changed(bus);
            }
            ADDR_PCICR => {
                self.pcicr = value & 0x07;
                self.mega_ext_config_changed(bus);
            }
            ADDR_EICRA => {
                self.eicra = value;
                self.mega_ext_config_changed(bus);
            }
            ADDR_EICRB => {
                self.mx.eicrb = value;
                self.mega_ext_config_changed(bus);
            }
            ADDR_PCMSK0..=ADDR_PCMSK2 => {
                self.pcmsk[usize::from(addr - ADDR_PCMSK0)] = value;
                self.mega_ext_config_changed(bus);
            }
            _ => {}
        }
    }
}
