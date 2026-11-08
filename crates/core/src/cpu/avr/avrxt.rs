// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! AVRxt core services: the data-space map, Configuration Change Protection
//! and the CPU interrupt controller (CPUINT) of the megaAVR 0-series and the
//! tinyAVR 0/1-series.
//!
//! Sources: Microchip ATmega4808/4809 data sheet DS40002173 (§8 "AVR CPU",
//! §12 "CPUINT", §7 "Memories"), and the ATmega4809 ATDF from the Microchip
//! ATmega DFP for register offsets and reset values.
//!
//! Interrupts are numbered as the data sheet's vector table (RESET = 0,
//! TCB3_INT = 36 on the ATmega4809): a peripheral that raises vector `n`
//! pends bit `n` of `pending_irq`, and the vector lives at byte `n * size`,
//! where `size` is 4 (a JMP) on parts with more than 8 KB of flash and 2 (an
//! RJMP) below. This differs from the classic ATmega path in `avr.rs`, which
//! counts vectors from 1.
//!
//! Unlike the classic core, taking an interrupt does NOT clear SREG.I and RETI
//! does not set it. CPUINT.STATUS.LVL0EX / LVL1EX say a level is being
//! served: while LVL0EX is set no other level-0 interrupt is taken, and a
//! level-1 interrupt (the vector in LVL1VEC) can still preempt it. RETI clears
//! the highest active level. Level-0 priority is static (the lowest vector
//! number wins) unless LVL0PRI moves the start point; CTRLA.LVL0RR makes it
//! round-robin. Not modelled: the NMI level (CRCSCAN), CTRLA.IVSEL (vectors
//! in the boot section) and CTRLA.CVT (compact vector table); a write that
//! sets IVSEL or CVT is refused rather than silently ignored.

use super::{Avr, AvrModel, AVR_IO_MIRROR_BASE};
use crate::{Bus, SimResult, SimulationError};

/// `CPU.CCP`, data space 0x0034.
pub(super) const ADDR_CCP: u16 = 0x0034;
/// Signature that unlocks the protected I/O registers for four instructions.
const CCP_IOREG: u8 = 0xD8;
/// Signature that unlocks SPM / NVMCTRL commands for four instructions.
const CCP_SPM: u8 = 0x9D;
/// CPUINT block (CTRLA, STATUS, LVL0PRI, LVL1VEC), data space 0x0110.
const CPUINT_BASE: u16 = 0x0110;
const CPUINT_CTRLA_LVL0RR: u8 = 1 << 0;
const CPUINT_CTRLA_CVT: u8 = 1 << 5;
const CPUINT_CTRLA_IVSEL: u8 = 1 << 6;
const CPUINT_STATUS_LVL0EX: u8 = 1 << 0;
const CPUINT_STATUS_LVL1EX: u8 = 1 << 1;

/// Per-core AVRxt state that has no home in the classic `Avr` fields.
#[derive(Debug, Clone, Default)]
pub(super) struct XtState {
    /// Instructions left in an open CCP IOREG window (0 = closed).
    ccp_ioreg_left: u8,
    /// `CPUINT.CTRLA`, `STATUS`, `LVL0PRI`, `LVL1VEC`.
    cpuint: [u8; 4],
}

impl XtState {
    /// One instruction retired: the CCP window counts down.
    #[inline]
    pub(super) fn retire(&mut self) {
        self.ccp_ioreg_left = self.ccp_ioreg_left.saturating_sub(1);
    }
}

impl AvrModel {
    /// Where flash appears in the AVRxt data space (LD/ST, avr-gcc `.rodata`).
    pub(super) fn mapped_flash_base(self) -> u16 {
        match self {
            AvrModel::Atmega4809 => 0x4000,
            _ => 0x8000,
        }
    }

    /// Last data-space address that belongs to I/O and system memories
    /// (NVMCTRL, SIGROW, FUSE, USERROW sit above 0x0FFF on the ATmega4809).
    pub(super) fn avrxt_io_end(self) -> u16 {
        match self {
            AvrModel::Atmega4809 => 0x13FF,
            _ => 0x0FFF,
        }
    }

    /// A part whose descriptor declares every I/O window it claims: an I/O
    /// access no window covers is a fault, not a store into scratch memory.
    pub(super) fn avrxt_strict_io(self) -> bool {
        self == AvrModel::Atmega4809
    }

    /// Bytes per interrupt vector: a JMP above 8 KB of flash, an RJMP below.
    pub(super) fn vector_bytes(self) -> u32 {
        if self.flash_size() > 8 * 1024 {
            4
        } else {
            2
        }
    }

    /// The ATmega4809 VPORT block (0x0000..=0x001B, four bytes per port)
    /// aliases DIR / OUT / IN / INTFLAGS of the full PORT window
    /// (0x0400 + 0x20 * port), so both views share one register file.
    fn vport_alias(self, addr: u16) -> Option<u16> {
        if self != AvrModel::Atmega4809 || addr >= 0x1C {
            return None;
        }
        let port = addr / 4;
        let reg = [0x00, 0x04, 0x08, 0x09][usize::from(addr % 4)];
        Some(0x0400 + 0x20 * port + reg)
    }

    /// Registers that ignore a write unless a CCP IOREG window is open
    /// (DS40002173 §8.5.7 / per-register "protected" notes): CLKCTRL's
    /// writable registers, RSTCTRL.SWRR and WDT.CTRLA / STATUS.
    fn ccp_protected(self, addr: u16) -> bool {
        self == AvrModel::Atmega4809
            && matches!(
                addr,
                0x0060..=0x0062 | 0x0070..=0x0072 | 0x0078 | 0x007C | 0x0041 | 0x0100 | 0x0101
            )
    }
}

impl Avr {
    /// AVRxt data-space read.
    pub(super) fn avrxt_data_read(&self, addr: u16, bus: &dyn Bus) -> SimResult<u8> {
        let model = self.model;
        if let Some(port_reg) = model.vport_alias(addr) {
            return self.avrxt_io_read(port_reg, bus);
        }
        match addr {
            0x003D => Ok(self.sp as u8),
            0x003E => Ok((self.sp >> 8) as u8),
            0x003F => Ok(self.sreg),
            ADDR_CCP => Ok(0),
            a @ CPUINT_BASE..=0x0113 => Ok(self.xt.cpuint[usize::from(a - CPUINT_BASE)]),
            a if a <= model.avrxt_io_end() => self.avrxt_io_read(a, bus),
            a if (self.sram_start..=self.ramend).contains(&a) => {
                Ok(self.sram[(a - self.sram_start) as usize])
            }
            a if a >= model.mapped_flash_base()
                && usize::from(a - model.mapped_flash_base()) < self.flash.len() =>
            {
                Ok(self.flash[usize::from(a - model.mapped_flash_base())])
            }
            _ => Err(SimulationError::MemoryViolation(u64::from(addr))),
        }
    }

    /// AVRxt data-space write. I/O writes go to the window the chip yaml maps
    /// there; SRAM writes update both the CPU's backing store and the bus.
    pub(super) fn avrxt_data_write(
        &mut self,
        addr: u16,
        value: u8,
        bus: &mut dyn Bus,
    ) -> SimResult<()> {
        let model = self.model;
        if let Some(port_reg) = model.vport_alias(addr) {
            return self.avrxt_io_write(port_reg, value, bus);
        }
        match addr {
            0x003D => {
                self.sp = (self.sp & 0xFF00) | u16::from(value);
                Ok(())
            }
            0x003E => {
                self.sp = (self.sp & 0x00FF) | (u16::from(value & 0x3F) << 8);
                Ok(())
            }
            0x003F => {
                self.sreg = value;
                Ok(())
            }
            ADDR_CCP => {
                // The window covers the next four instructions; `retire`
                // also runs for the STS/OUT that wrote the signature.
                match value {
                    CCP_IOREG => self.xt.ccp_ioreg_left = 5,
                    // NVMCTRL commands are not modelled; the SPM signature
                    // unlocks nothing here.
                    CCP_SPM => {}
                    _ => {}
                }
                Ok(())
            }
            a @ CPUINT_BASE..=0x0113 => self.cpuint_write(a - CPUINT_BASE, value),
            a if a <= model.avrxt_io_end() => {
                if model.ccp_protected(a) && self.xt.ccp_ioreg_left == 0 {
                    // Silicon ignores the write without the CCP signature.
                    return Ok(());
                }
                self.avrxt_io_write(a, value, bus)
            }
            a if (self.sram_start..=self.ramend).contains(&a) => {
                self.sram[(a - self.sram_start) as usize] = value;
                bus.write_u8(u64::from(a), value)?;
                Ok(())
            }
            _ => Err(SimulationError::MemoryViolation(u64::from(addr))),
        }
    }

    fn avrxt_io_read(&self, addr: u16, bus: &dyn Bus) -> SimResult<u8> {
        let mirror = AVR_IO_MIRROR_BASE + u64::from(addr);
        if bus.has_mmio_window(mirror) {
            bus.read_u8(mirror)
        } else if self.model.avrxt_strict_io() && !Self::avrxt_gpior(addr) {
            Err(SimulationError::MemoryViolation(u64::from(addr)))
        } else {
            Ok(self.tiny_io[usize::from(addr)])
        }
    }

    fn avrxt_io_write(&mut self, addr: u16, value: u8, bus: &mut dyn Bus) -> SimResult<()> {
        let mirror = AVR_IO_MIRROR_BASE + u64::from(addr);
        if bus.has_mmio_window(mirror) {
            bus.write_u8(mirror, value)
        } else if self.model.avrxt_strict_io() && !Self::avrxt_gpior(addr) {
            Err(SimulationError::MemoryViolation(u64::from(addr)))
        } else {
            self.tiny_io[usize::from(addr)] = value;
            Ok(())
        }
    }

    /// GPIOR0..3 (0x001C..=0x001F): plain general-purpose registers.
    fn avrxt_gpior(addr: u16) -> bool {
        (0x001C..=0x001F).contains(&addr)
    }

    fn cpuint_write(&mut self, reg: u16, value: u8) -> SimResult<()> {
        match reg {
            0 => {
                if self.xt.ccp_ioreg_left == 0 {
                    return Ok(());
                }
                if value & (CPUINT_CTRLA_CVT | CPUINT_CTRLA_IVSEL) != 0 {
                    return Err(SimulationError::NotImplemented(format!(
                        "CPUINT.CTRLA {value:#04x}: compact vector table / boot-section \
                         vectors are not modelled"
                    )));
                }
                self.xt.cpuint[0] = value & CPUINT_CTRLA_LVL0RR;
            }
            // STATUS is set by vector entry and cleared by RETI only.
            1 => {}
            2 | 3 => self.xt.cpuint[usize::from(reg)] = value,
            _ => {}
        }
        Ok(())
    }

    /// Take the interrupt CPUINT would serve now, if any.
    pub(super) fn try_take_irq_avrxt(&mut self, bus: &mut dyn Bus) -> SimResult<bool> {
        if !self.flag_i() || self.pending_irq & !1 == 0 {
            return Ok(false);
        }
        let status = self.xt.cpuint[1];
        if status & CPUINT_STATUS_LVL1EX != 0 {
            return Ok(false);
        }
        let lvl1 = u32::from(self.xt.cpuint[3]);
        let vec = if lvl1 != 0 && self.pending_irq & (1u64 << lvl1) != 0 {
            self.xt.cpuint[1] |= CPUINT_STATUS_LVL1EX;
            lvl1
        } else if status & CPUINT_STATUS_LVL0EX != 0 {
            return Ok(false);
        } else {
            let mut level0 = self.pending_irq & !1;
            if lvl1 != 0 {
                level0 &= !(1u64 << lvl1);
            }
            let Some(vec) = Self::level0_winner(level0, u32::from(self.xt.cpuint[2])) else {
                return Ok(false);
            };
            self.xt.cpuint[1] |= CPUINT_STATUS_LVL0EX;
            if self.xt.cpuint[0] & CPUINT_CTRLA_LVL0RR != 0 {
                self.xt.cpuint[2] = vec as u8;
            }
            vec
        };
        self.pending_irq &= !(1u64 << vec);
        self.push_pc(bus)?;
        self.pc = vec * self.model.vector_bytes();
        Ok(true)
    }

    /// Level-0 priority: with LVL0PRI = 0 the lowest vector number wins;
    /// otherwise the search starts at LVL0PRI + 1 and wraps (DS40002173
    /// §12.3.3.1, "Static" / "Round Robin" scheduling).
    fn level0_winner(pending: u64, lvl0pri: u32) -> Option<u32> {
        if pending == 0 {
            return None;
        }
        if lvl0pri == 0 {
            return Some(pending.trailing_zeros());
        }
        let start = (lvl0pri + 1).min(63);
        let high = pending & (!0u64 << start);
        Some(if high != 0 {
            high.trailing_zeros()
        } else {
            pending.trailing_zeros()
        })
    }

    /// RETI on AVRxt: leave the highest active level; SREG.I is untouched.
    pub(super) fn reti_avrxt(&mut self) {
        let status = &mut self.xt.cpuint[1];
        if *status & CPUINT_STATUS_LVL1EX != 0 {
            *status &= !CPUINT_STATUS_LVL1EX;
        } else {
            *status &= !CPUINT_STATUS_LVL0EX;
        }
    }

    pub(super) fn reset_avrxt(&mut self) {
        self.xt = XtState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_priority_takes_the_lowest_vector() {
        let pending = (1u64 << 36) | (1u64 << 38);
        assert_eq!(Avr::level0_winner(pending, 0), Some(36));
    }

    #[test]
    fn lvl0pri_moves_the_start_point_and_wraps() {
        let pending = (1u64 << 12) | (1u64 << 36) | (1u64 << 38);
        assert_eq!(Avr::level0_winner(pending, 36), Some(38));
        assert_eq!(Avr::level0_winner(pending, 38), Some(12));
    }

    #[test]
    fn vport_aliases_the_port_window() {
        let m = AvrModel::Atmega4809;
        // VPORTE.OUT (0x0011) is PORTE.OUT (0x0484); VPORTB.IN is PORTB.IN.
        assert_eq!(m.vport_alias(0x0011), Some(0x0484));
        assert_eq!(m.vport_alias(0x0006), Some(0x0428));
        assert_eq!(m.vport_alias(0x001C), None);
        assert_eq!(AvrModel::Attiny404.vport_alias(0x0011), None);
    }

    #[test]
    fn the_4809_vector_is_a_jmp_and_flash_maps_at_0x4000() {
        assert_eq!(AvrModel::Atmega4809.vector_bytes(), 4);
        assert_eq!(AvrModel::Atmega4809.mapped_flash_base(), 0x4000);
        assert_eq!(AvrModel::Attiny202.vector_bytes(), 2);
        assert_eq!(AvrModel::Attiny404.mapped_flash_base(), 0x8000);
    }
}
