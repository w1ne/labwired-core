// SPDX-License-Identifier: MIT

//! PL190 VIC register and vectored-interrupt behavior used by Renode's
//! `platforms/cpus/lpc2294.repl`.
//!
//! Scope follows Renode's `PL190_VIC` model: 32 interrupt inputs, IRQ/FIQ
//! selection, set/clear enable and software-interrupt registers, 16 vectored
//! slots, vector acknowledge/EOI, and peripheral ID registers. LPC2294
//! peripheral-to-VIC input wiring is not part of this slice yet.

use crate::{Peripheral, SimResult};
use std::sync::{Mutex, MutexGuard};

const INPUTS: usize = 32;
const VECTOR_SLOTS: usize = 16;

#[derive(Debug, Clone)]
struct Pl190State {
    enabled: u32,
    fiq_select: u32,
    software: u32,
    pins: u32,
    default_vector: u32,
    vector_address: [u32; VECTOR_SLOTS],
    vector_control: [u32; VECTOR_SLOTS],
    pending_irq: Vec<usize>,
    serviced_irq: Vec<usize>,
}

impl Default for Pl190State {
    fn default() -> Self {
        Self {
            enabled: 0,
            fiq_select: 0,
            software: 0,
            pins: 0,
            default_vector: 0,
            vector_address: [0; VECTOR_SLOTS],
            vector_control: [0; VECTOR_SLOTS],
            pending_irq: Vec::new(),
            serviced_irq: Vec::new(),
        }
    }
}

impl Pl190State {
    fn active(&self) -> u32 {
        (self.pins | self.software) & self.enabled
    }

    fn irq_status(&self) -> u32 {
        self.active() & !self.fiq_select
    }

    fn fiq_status(&self) -> u32 {
        self.active() & self.fiq_select
    }

    fn vector_slot(&self, source: usize) -> Option<usize> {
        self.vector_control
            .iter()
            .position(|control| control & (1 << 5) != 0 && (control & 0x1F) as usize == source)
    }

    fn priority(&self, source: usize) -> usize {
        self.vector_slot(source).unwrap_or(VECTOR_SLOTS)
    }

    fn refresh(&mut self) {
        let active_irq = self.irq_status();
        self.pending_irq
            .retain(|source| active_irq & (1u32 << source) != 0);
        while self
            .serviced_irq
            .last()
            .is_some_and(|source| active_irq & (1u32 << source) == 0)
        {
            self.serviced_irq.pop();
        }

        for source in 0..INPUTS {
            let bit = 1u32 << source;
            if active_irq & bit != 0
                && !self.pending_irq.contains(&source)
                && !self.serviced_irq.contains(&source)
            {
                self.pending_irq.push(source);
            }
        }
        let priorities: [usize; INPUTS] = std::array::from_fn(|source| self.priority(source));
        self.pending_irq
            .sort_by_key(|source| (priorities[*source], *source));
    }

    fn irq_line(&self) -> bool {
        self.pending_irq
            .first()
            .is_some_and(|source| self.irq_status() & (1u32 << source) != 0)
    }

    fn fiq_line(&self) -> bool {
        self.fiq_status() != 0
    }

    fn acknowledge(&mut self) -> u32 {
        let Some(source) = self.pending_irq.first().copied() else {
            return 0;
        };
        self.pending_irq.remove(0);
        self.serviced_irq.push(source);
        self.vector_slot(source)
            .map(|slot| self.vector_address[slot])
            .unwrap_or(self.default_vector)
    }

    fn read_register(&mut self, offset: u64, side_effects: bool) -> u32 {
        match offset {
            0x000 => self.irq_status(),
            0x004 => self.fiq_status(),
            // Renode declares this register address but does not define a
            // register model for it; preserve its read-as-zero behavior.
            0x008 => 0,
            0x00C => self.fiq_select,
            0x010 => self.enabled,
            0x014 | 0x01C => 0,
            0x018 => self.software,
            0x020 => 0,
            0x030 if side_effects => self.acknowledge(),
            0x030 => self
                .pending_irq
                .first()
                .and_then(|source| {
                    self.vector_slot(*source)
                        .map(|slot| self.vector_address[slot])
                        .or(Some(self.default_vector))
                })
                .unwrap_or(0),
            0x034 => self.default_vector,
            0x100..=0x13C if offset & 3 == 0 => {
                self.vector_address[((offset - 0x100) / 4) as usize]
            }
            0x200..=0x23C if offset & 3 == 0 => {
                self.vector_control[((offset - 0x200) / 4) as usize]
            }
            0xFE0 => 0x90,
            0xFE4 => 0x11,
            0xFE8 => 0x10,
            0xFEC => 0,
            // The PrimeCell ID registers are not defined by Renode's model.
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32) {
        match offset {
            0x00C => self.fiq_select = value,
            0x010 => self.enabled |= value,
            0x014 => self.enabled &= !value,
            0x018 => self.software |= value,
            0x01C => self.software &= !value,
            0x030 => {
                self.serviced_irq.pop();
            }
            0x034 => self.default_vector = value,
            0x100..=0x13C if offset & 3 == 0 => {
                self.vector_address[((offset - 0x100) / 4) as usize] = value;
            }
            0x200..=0x23C if offset & 3 == 0 => {
                self.vector_control[((offset - 0x200) / 4) as usize] = value & 0x3F;
            }
            _ => return,
        }
        self.refresh();
    }

    fn write_byte(&mut self, offset: u64, value: u8) {
        let register = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        let lane = u32::from(value) << shift;
        if matches!(register, 0x010 | 0x014 | 0x018 | 0x01C | 0x030) {
            self.write_register(register, lane);
            return;
        }
        let mask = 0xFFu32 << shift;
        let previous = self.read_register(register, false);
        self.write_register(register, (previous & !mask) | lane);
    }
}

/// ARM PrimeCell PL190 VIC with the register behavior exercised by the
/// imported LPC2294 platform.
#[derive(Debug, Default)]
pub struct Pl190Vic {
    state: Mutex<Pl190State>,
}

impl Pl190Vic {
    pub const SIZE: u64 = 0x1000;

    pub fn new() -> Self {
        Self::default()
    }

    fn lock_state(&self) -> MutexGuard<'_, Pl190State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Reconcile one external peripheral interrupt input as a level.
    pub fn set_input_level(&self, source: usize, level: bool) {
        if source >= INPUTS {
            return;
        }
        let mut state = self.lock_state();
        let bit = 1u32 << source;
        if level {
            state.pins |= bit;
        } else {
            state.pins &= !bit;
        }
        state.refresh();
    }
}

impl Peripheral for Pl190Vic {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        // Byte reads share the register's side effect. Normal ARM word reads
        // take the read_u32 override below and acknowledge only once.
        let value = self.lock_state().read_register(offset & !3, true);
        Ok((value >> ((offset & 3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.lock_state().write_byte(offset, value);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset & 3 == 0 {
            Ok(self.lock_state().read_register(offset, true))
        } else {
            let mut value = 0;
            for byte in 0..4 {
                value |= u32::from(self.read(offset + byte)?) << (byte * 8);
            }
            Ok(value)
        }
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.lock_state().write_register(offset, value);
        } else {
            for byte in 0..4 {
                self.write(offset + byte, (value >> (byte * 8)) as u8)?;
            }
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let value = self.lock_state().read_register(offset & !3, false);
        Some((value >> ((offset & 3) * 8)) as u8)
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.lock_state().irq_line())
    }

    fn arm_irq_fiq_levels(&self) -> Option<(bool, bool)> {
        let state = self.lock_state();
        Some((state.irq_line(), state.fiq_line()))
    }

    fn on_node_reset(&mut self) {
        *self.lock_state() = Pl190State::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(vic: &mut Pl190Vic, offset: u64, value: u32) {
        vic.write_u32(offset, value).unwrap();
    }

    fn read(vic: &Pl190Vic, offset: u64) -> u32 {
        vic.read_u32(offset).unwrap()
    }

    #[test]
    fn software_irq_is_enabled_vectored_acknowledged_and_completed() {
        let mut vic = Pl190Vic::new();
        assert_eq!(vic.arm_irq_fiq_levels(), Some((false, false)));

        write(&mut vic, 0x100, 0x1234_5678);
        write(&mut vic, 0x200, (1 << 5) | 5);
        write(&mut vic, 0x010, 1 << 5);
        write(&mut vic, 0x018, 1 << 5);

        assert_eq!(read(&vic, 0x000), 1 << 5);
        assert_eq!(vic.arm_irq_fiq_levels(), Some((true, false)));
        assert_eq!(read(&vic, 0x030), 0x1234_5678);
        assert_eq!(vic.arm_irq_fiq_levels(), Some((false, false)));

        write(&mut vic, 0x01C, 1 << 5);
        write(&mut vic, 0x030, 0);
        assert_eq!(read(&vic, 0x000), 0);
        assert_eq!(read(&vic, 0x030), 0);
    }

    #[test]
    fn fiq_selection_bypasses_the_vectored_irq_output() {
        let mut vic = Pl190Vic::new();
        write(&mut vic, 0x010, 1 << 7);
        write(&mut vic, 0x00C, 1 << 7);
        write(&mut vic, 0x018, 1 << 7);

        assert_eq!(read(&vic, 0x000), 0);
        assert_eq!(read(&vic, 0x004), 1 << 7);
        assert_eq!(vic.arm_irq_fiq_levels(), Some((false, true)));
    }

    #[test]
    fn source_enable_clear_and_ids_follow_renode_register_window() {
        let mut vic = Pl190Vic::new();
        write(&mut vic, 0x010, (1 << 2) | (1 << 4));
        write(&mut vic, 0x018, 1 << 2);
        assert_eq!(read(&vic, 0x000), 1 << 2);
        write(&mut vic, 0x014, 1 << 2);
        assert_eq!(read(&vic, 0x000), 0);
        assert_eq!(read(&vic, 0xFE0), 0x90);
        assert_eq!(read(&vic, 0xFE4), 0x11);
        assert_eq!(read(&vic, 0xFE8), 0x10);
        assert_eq!(read(&vic, 0xFEC), 0);
        assert_eq!(read(&vic, 0xFF0), 0);
        assert_eq!(read(&vic, 0x008), 0);
        assert_eq!(read(&vic, 0x020), 0);
    }
}
