//! WCH PFIC register block and interrupt arbitration.
//!
//! Offsets are relative to a future PFIC mapping at `0xE000_E000`. The status,
//! enable, pending, active, priority, and SCTLR addresses are those listed in
//! the task brief. The PFIC-to-IRQ-number mapping and priority ordering were
//! not verified against CH32V003RM; this model uses zero-based API source IDs
//! and treats larger four-bit priority values as more urgent.

use crate::{Peripheral, SimResult};

/// Four core and 23 peripheral sources, as specified by the task brief.
pub const PFIC_SOURCE_COUNT: usize = 27;

const ISR1: u64 = 0x000;
const ISR2: u64 = 0x004;
const IENR1: u64 = 0x100;
const IENR2: u64 = 0x104;
const IRER1: u64 = 0x180;
const IRER2: u64 = 0x184;
const IPSR1: u64 = 0x200;
const IPSR2: u64 = 0x204;
const IPRR1: u64 = 0x280;
const IPRR2: u64 = 0x284;
const IACTR1: u64 = 0x300;
const IACTR2: u64 = 0x304;
const IPRIOR_BASE: u64 = 0x400;
const SCTLR: u64 = 0xD10;
const SOURCE_MASK: u64 = (1u64 << PFIC_SOURCE_COUNT) - 1;

#[derive(Debug, Clone, Copy, serde::Serialize)]
struct VectorFreeEntry {
    source: u8,
    handler: u32,
}

/// Self-contained model of WCH's programmable fast interrupt controller.
///
/// Source IDs in this API are zero-based 0..26. The source-to-bit mapping is a
/// model convention because the CH32V003 IRQ-number allocation was unavailable
/// for verification. Register bit operations use that same source numbering.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Pfic {
    enabled: u64,
    pending: u64,
    active: u64,
    priorities: [u8; PFIC_SOURCE_COUNT],
    sctlr: u32,
    nesting_enabled: bool,
    active_stack: Vec<u8>,
    vector_free_entries: Vec<Option<VectorFreeEntry>>,
}

impl Default for Pfic {
    fn default() -> Self {
        Self {
            enabled: 0,
            pending: 0,
            active: 0,
            priorities: [0; PFIC_SOURCE_COUNT],
            sctlr: 0,
            nesting_enabled: false,
            active_stack: Vec::new(),
            vector_free_entries: Vec::new(),
        }
    }
}

impl Pfic {
    pub fn new() -> Self {
        Self::default()
    }

    fn valid_source(source: u8) -> bool {
        usize::from(source) < PFIC_SOURCE_COUNT
    }

    fn bit(source: u8) -> u64 {
        1u64 << source
    }

    /// Enable nested interrupt arbitration. The integration layer can drive
    /// this when the CPU's interrupt-nesting control is modeled.
    pub fn set_nesting_enabled(&mut self, enabled: bool) {
        self.nesting_enabled = enabled;
    }

    pub fn set_pending(&mut self, source: u8) -> bool {
        if !Self::valid_source(source) {
            return false;
        }
        self.pending |= Self::bit(source);
        true
    }

    pub fn clear_pending(&mut self, source: u8) -> bool {
        if !Self::valid_source(source) {
            return false;
        }
        self.pending &= !Self::bit(source);
        true
    }

    pub fn is_enabled(&self, source: u8) -> bool {
        Self::valid_source(source) && self.enabled & Self::bit(source) != 0
    }

    pub fn is_pending(&self, source: u8) -> bool {
        Self::valid_source(source) && self.pending & Self::bit(source) != 0
    }

    pub fn is_active(&self, source: u8) -> bool {
        Self::valid_source(source) && self.active & Self::bit(source) != 0
    }

    /// Return the highest-priority enabled, pending and inactive source.
    /// Equal priorities use ascending source ID. At most two levels can be
    /// active at once; a nested source must outrank the currently active one.
    pub fn pending_source(&self) -> Option<u8> {
        if self.active_stack.len() >= 2 {
            return None;
        }
        let running = self.active_stack.last().copied();
        (0..PFIC_SOURCE_COUNT)
            .filter(|source| {
                let bit = 1u64 << source;
                if self.enabled & self.pending & !self.active & bit == 0 {
                    return false;
                }
                match running {
                    Some(_) if !self.nesting_enabled => false,
                    Some(top) => self.priorities[*source] > self.priorities[usize::from(top)],
                    None => true,
                }
            })
            .max_by_key(|source| (self.priorities[*source], core::cmp::Reverse(*source)))
            .map(|source| source as u8)
    }

    /// Acknowledge the selected source: clear pending, mark active, and return
    /// its zero-based source ID for the CPU interrupt path.
    pub fn acknowledge(&mut self) -> Option<u8> {
        let source = self.pending_source()?;
        let bit = Self::bit(source);
        self.pending &= !bit;
        self.active |= bit;
        self.active_stack.push(source);
        Some(source)
    }

    /// Complete the most recently acknowledged source (nested interrupts are
    /// completed in LIFO order).
    pub fn complete(&mut self, source: u8) -> bool {
        if self.active_stack.last().copied() != Some(source) {
            return false;
        }
        self.active_stack.pop();
        self.active &= !Self::bit(source);
        true
    }

    /// Configure an abstract vector-table-free entry. VTF register offsets and
    /// slot count were not present in the brief, so those details remain outside
    /// the MMIO map until verified. The API lets a future bus integration query
    /// the handler without inventing those offsets.
    pub fn configure_vector_free_entry(&mut self, slot: usize, source: u8, handler: u32) -> bool {
        if !Self::valid_source(source) {
            return false;
        }
        if self.vector_free_entries.len() <= slot {
            self.vector_free_entries.resize(slot + 1, None);
        }
        self.vector_free_entries[slot] = Some(VectorFreeEntry { source, handler });
        true
    }

    pub fn vector_free_handler(&self, source: u8) -> Option<u32> {
        Self::valid_source(source).then_some(())?;
        self.vector_free_entries
            .iter()
            .flatten()
            .find(|entry| entry.source == source)
            .map(|entry| entry.handler)
    }

    fn valid_mask(word: u64) -> u32 {
        let first_source = word * 32;
        if first_source >= PFIC_SOURCE_COUNT as u64 {
            0
        } else {
            let remaining = PFIC_SOURCE_COUNT as u64 - first_source;
            if remaining >= 32 {
                u32::MAX
            } else {
                ((1u64 << remaining) - 1) as u32
            }
        }
    }

    fn read_word(bits: u64, word: u64) -> u32 {
        ((bits >> (word * 32)) as u32) & Self::valid_mask(word)
    }

    fn write_set(bits: &mut u64, word: u64, value: u32) {
        *bits |= u64::from(value & Self::valid_mask(word)) << (word * 32);
        *bits &= SOURCE_MASK;
    }

    fn write_clear(bits: &mut u64, word: u64, value: u32) {
        *bits &= !(u64::from(value & Self::valid_mask(word)) << (word * 32));
        *bits &= SOURCE_MASK;
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            ISR1 => Self::read_word(self.enabled, 0),
            ISR2 => Self::read_word(self.enabled, 1),
            IENR1 | IENR2 | IRER1 | IRER2 | IPRR1 | IPRR2 => 0,
            IPSR1 => Self::read_word(self.pending, 0),
            IPSR2 => Self::read_word(self.pending, 1),
            IACTR1 => Self::read_word(self.active, 0),
            IACTR2 => Self::read_word(self.active, 1),
            offset
                if (IPRIOR_BASE..IPRIOR_BASE + 4 * PFIC_SOURCE_COUNT as u64).contains(&offset)
                    && offset % 4 == 0 =>
            {
                self.priorities[((offset - IPRIOR_BASE) / 4) as usize] as u32
            }
            SCTLR => self.sctlr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            IENR1 => Self::write_set(&mut self.enabled, 0, value),
            IENR2 => Self::write_set(&mut self.enabled, 1, value),
            IRER1 => Self::write_clear(&mut self.enabled, 0, value),
            IRER2 => Self::write_clear(&mut self.enabled, 1, value),
            IPSR1 => Self::write_set(&mut self.pending, 0, value),
            IPSR2 => Self::write_set(&mut self.pending, 1, value),
            IPRR1 => Self::write_clear(&mut self.pending, 0, value),
            IPRR2 => Self::write_clear(&mut self.pending, 1, value),
            offset
                if (IPRIOR_BASE..IPRIOR_BASE + 4 * PFIC_SOURCE_COUNT as u64).contains(&offset)
                    && offset % 4 == 0 =>
            {
                self.priorities[((offset - IPRIOR_BASE) / 4) as usize] = (value & 0xF) as u8;
            }
            SCTLR => self.sctlr = value,
            // Status registers and reserved addresses are read-only / inert.
            _ => {}
        }
        self.enabled &= SOURCE_MASK;
        self.pending &= SOURCE_MASK;
        self.active &= SOURCE_MASK;
    }
}

impl Peripheral for Pfic {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let shift = (offset & 3) * 8;
        Ok(((self.read_reg(reg) >> shift) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset & 3) * 8;
        let byte_mask = 0xFFu32 << shift;
        let word = (self.read_reg(reg) & !byte_mask) | (u32::from(value) << shift);
        self.write_reg(reg, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value);
            return Ok(());
        }
        for (index, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + index as u64, *byte)?;
        }
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_set_and_clear_registers_update_status() {
        let mut pfic = Pfic::new();
        pfic.write_u32(IENR1, (1 << 3) | (1 << 6)).unwrap();
        assert_eq!(pfic.read_u32(ISR1).unwrap(), (1 << 3) | (1 << 6));
        pfic.write_u32(IENR1, 1).unwrap();
        assert_eq!(pfic.read_u32(ISR1).unwrap(), (1 << 3) | (1 << 6) | 1);
        pfic.write_u32(IRER1, 1 << 3).unwrap();
        assert_eq!(pfic.read_u32(ISR1).unwrap(), (1 << 6) | 1);
    }

    #[test]
    fn pending_set_and_clear_registers_update_status() {
        let mut pfic = Pfic::new();
        pfic.write_u32(IPSR1, (1 << 2) | (1 << 5)).unwrap();
        assert_eq!(pfic.read_u32(IPSR1).unwrap(), (1 << 2) | (1 << 5));
        pfic.write_u32(IPRR1, 1 << 2).unwrap();
        assert_eq!(pfic.read_u32(IPSR1).unwrap(), 1 << 5);
    }

    #[test]
    fn priority_selects_highest_pending_enabled_source() {
        let mut pfic = Pfic::new();
        pfic.write_u32(IENR1, (1 << 1) | (1 << 4) | (1 << 7))
            .unwrap();
        pfic.write_u32(IPSR1, (1 << 1) | (1 << 4) | (1 << 7))
            .unwrap();
        pfic.write_u32(IPRIOR_BASE + 4, 3).unwrap();
        pfic.write_u32(IPRIOR_BASE + 16, 12).unwrap();
        pfic.write_u32(IPRIOR_BASE + 28, 7).unwrap();
        assert_eq!(pfic.read_u32(IPRIOR_BASE + 16).unwrap(), 12);
        assert_eq!(pfic.pending_source(), Some(4));
        assert_eq!(pfic.acknowledge(), Some(4));
        assert_eq!(pfic.read_u32(IACTR1).unwrap(), 1 << 4);
    }

    #[test]
    fn higher_priority_interrupt_nests_to_two_active_levels() {
        let mut pfic = Pfic::new();
        pfic.set_nesting_enabled(true);
        pfic.write_u32(IENR1, (1 << 1) | (1 << 2)).unwrap();
        pfic.write_u32(IPRIOR_BASE + 4, 2).unwrap();
        pfic.write_u32(IPRIOR_BASE + 8, 9).unwrap();
        pfic.set_pending(1);
        assert_eq!(pfic.acknowledge(), Some(1));
        pfic.set_pending(2);
        assert_eq!(pfic.acknowledge(), Some(2));
        assert_eq!(pfic.read_u32(IACTR1).unwrap(), (1 << 1) | (1 << 2));
        assert_eq!(pfic.pending_source(), None);
        assert!(pfic.complete(2));
        assert!(pfic.complete(1));
    }

    #[test]
    fn vector_free_handler_can_be_queried_by_source() {
        let mut pfic = Pfic::new();
        assert!(pfic.configure_vector_free_entry(0, 4, 0x2000_0100));
        assert_eq!(pfic.vector_free_handler(4), Some(0x2000_0100));
        assert_eq!(pfic.vector_free_handler(5), None);
        assert!(!pfic.configure_vector_free_entry(1, 27, 0x2000_0200));
    }
}
