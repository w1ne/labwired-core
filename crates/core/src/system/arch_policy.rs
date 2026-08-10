// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The one place that decides what silicon a run models.
//!
//! Two decisions live here, and nowhere else:
//!
//! * [`machine_family`] — what a *chip* becomes, read from its declared `arch:`.
//! * [`elf_arch`] — what an *ELF* is, read from its `e_machine` field.
//!
//! Both were previously duplicated, and both duplicates had drifted. The CLI
//! refused a chip that declared no architecture; the browser ran it as a
//! Cortex-M. The in-core ELF decoder bailed on an unrecognised machine; the
//! `loader` crate warned and returned [`crate::Arch::Unknown`], spelling
//! EM_XTENSA as the literal `94`. A simulator sold as an oracle cannot hold two
//! opinions about what processor the firmware is, so these are functions rather
//! than a convention, and `crate::tests::one_arch_policy` fails if a second
//! copy grows back.

use labwired_config::{Arch, ChipDescriptor};

/// The machine family a chip is built as.
///
/// This is deliberately coarser than [`Arch`]: it answers "which builder", not
/// "which core". Callers still choose among variants *within* a family (an
/// ESP32-S3 boots differently from an ESP32), because that is a build detail
/// and not an architecture decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineFamily {
    CortexM,
    RiscV,
    Xtensa,
    Avr,
}

/// What machine family this chip is built as, or an error naming why not.
///
/// A chip that declares no architecture is **refused**. It is never guessed:
/// guessing is what let an `arch: unknown` chip execute as a Cortex-M in the
/// browser, producing confident output about silicon nobody had modelled.
pub fn machine_family(chip: &ChipDescriptor) -> anyhow::Result<MachineFamily> {
    match chip.arch {
        Arch::Arm => Ok(MachineFamily::CortexM),
        Arch::RiscV => Ok(MachineFamily::RiscV),
        Arch::Xtensa => Ok(MachineFamily::Xtensa),
        Arch::Avr => Ok(MachineFamily::Avr),
        Arch::Unknown => anyhow::bail!(
            "chip '{}' does not declare a known architecture (`arch:` must be arm, riscv, xtensa, or avr)",
            chip.name
        ),
    }
}

/// The architecture an ELF's `e_machine` field names, or `None` if this engine
/// models no machine for it.
///
/// `None` is the single meaning of "unrecognised". What to *do* about it is the
/// caller's business — the in-core node builder treats it as fatal, the loader
/// records it as [`crate::Arch::Unknown`] for a caller that will check later —
/// but neither decides any longer what the bytes mean.
pub fn elf_arch(e_machine: u16) -> Option<crate::Arch> {
    match e_machine {
        goblin::elf::header::EM_ARM => Some(crate::Arch::Arm),
        goblin::elf::header::EM_RISCV => Some(crate::Arch::RiscV),
        goblin::elf::header::EM_XTENSA => Some(crate::Arch::XtensaLx7),
        83 => Some(crate::Arch::Avr), // EM_AVR
        _ => None,
    }
}
