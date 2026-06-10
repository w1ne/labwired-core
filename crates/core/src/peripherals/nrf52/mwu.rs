// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 MWU (Memory Watch Unit).
//!
//! Source: nRF52840 PS rev 1.7 §6.12 (MWU). Debug peripheral that
//! generates events on memory accesses. Register-surface only —
//! no actual access monitoring.
//!
//! Silicon note (confirmed on nRF52840 rev 3):
//! REGION[0..3].START/END (0x510..0x52C) and PREGION[0..1].START/END
//! (0x600..0x60C) read back as 0 regardless of what was written — these
//! are effectively write-only configuration registers on real silicon.
//! REGIONEN / REGIONENSET / REGIONENCLR at 0x500/0x504/0x508 do
//! round-trip correctly.
//! EVENTS_* at 0x100..0x17C: write-1 ignored, write-0 clears.

use crate::{Peripheral, SimResult};

// EVENTS range: 0x100..0x17C (region[n] read/write/noaccess events)
const OFF_EVENTS_FIRST: u64 = 0x100;
const OFF_EVENTS_LAST: u64 = 0x17C;

// Interrupt registers
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;

// REGIONEN block
const OFF_REGIONEN: u64 = 0x500;
const OFF_REGIONENSET: u64 = 0x504;
const OFF_REGIONENCLR: u64 = 0x508;

// REGION[0..3].START/END: write-only on silicon (reads return 0).
// These offsets accept writes but always read 0.
const OFF_REGION_FIRST: u64 = 0x510;
const OFF_REGION_LAST: u64 = 0x52C;

// PREGION[0..1] block: also write-only on silicon (0x600..0x60C range)
const OFF_PREGION_FIRST: u64 = 0x600;
const OFF_PREGION_LAST: u64 = 0x60C;

#[derive(Debug, Default)]
pub struct Nrf52Mwu {
    events: std::collections::BTreeMap<u64, u32>,
    inten: u32,
    regionen: u32,
    // REGION.START/END are accepted on write but not stored
    // (they read 0 on silicon — deliberately write-only).
}

impl Nrf52Mwu {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for Nrf52Mwu {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // Tasks: always 0
            0x000..=0x0FC if offset.is_multiple_of(4) => 0,
            // EVENTS: hardware-generated
            OFF_EVENTS_FIRST..=OFF_EVENTS_LAST if offset.is_multiple_of(4) => {
                self.events.get(&offset).copied().unwrap_or(0)
            }
            // Interrupts
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten,
            // REGIONEN block: silicon returns 0 (write-only in practice)
            OFF_REGIONEN | OFF_REGIONENSET | OFF_REGIONENCLR => 0,
            // REGION[0..3].START/END: write-only on silicon → always reads 0
            OFF_REGION_FIRST..=OFF_REGION_LAST if offset.is_multiple_of(4) => 0,
            // PREGION: same — read 0
            OFF_PREGION_FIRST..=OFF_PREGION_LAST if offset.is_multiple_of(4) => 0,
            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // Tasks: write-only trigger, no state change for reg-surface model
            0x000..=0x0FC if offset.is_multiple_of(4) => {}
            // EVENTS: SW write-1 ignored (falls through to the no-op default),
            // write-0 clears.
            OFF_EVENTS_FIRST..=OFF_EVENTS_LAST if offset.is_multiple_of(4) && value == 0 => {
                self.events.remove(&offset);
            }
            // Interrupts
            OFF_INTEN => self.inten = value,
            OFF_INTENSET => self.inten |= value,
            OFF_INTENCLR => self.inten &= !value,
            // REGIONEN: set / clear / direct
            OFF_REGIONEN => self.regionen = value,
            OFF_REGIONENSET => self.regionen |= value,
            OFF_REGIONENCLR => self.regionen &= !value,
            // REGION.START/END: accepted but not stored (write-only on silicon)
            OFF_REGION_FIRST..=OFF_REGION_LAST if offset.is_multiple_of(4) => {}
            // PREGION: accepted but not stored
            OFF_PREGION_FIRST..=OFF_PREGION_LAST if offset.is_multiple_of(4) => {}
            _ => {}
        }
        Ok(())
    }
}
