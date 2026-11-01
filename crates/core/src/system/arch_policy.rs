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
    CortexA,
    /// ARMv7-R real-time profile (Cortex-R5F) on the native system backend.
    CortexR,
    RiscV,
    Xtensa,
    Avr,
    Pic24,
    Pic18,
    Pic16,
    Aarch64,
    Rx,
}

/// What machine family this chip is built as, or an error naming why not.
///
/// A chip that declares no architecture is **refused**. It is never guessed:
/// guessing is what let an `arch: unknown` chip execute as a Cortex-M in the
/// browser, producing confident output about silicon nobody had modelled.
pub fn machine_family(chip: &ChipDescriptor) -> anyhow::Result<MachineFamily> {
    match chip.arch {
        Arch::Arm => match chip.core.as_deref().map(|core| core.trim().to_ascii_lowercase()) {
            Some(core) if core == "cortex-a7" => Ok(MachineFamily::CortexA),
            Some(core) if core == "cortex-r5f" => Ok(MachineFamily::CortexR),
            Some(core) if core.starts_with("cortex-r") => anyhow::bail!(
                "chip '{}': unsupported ARM real-time core '{core}'; only cortex-r5f has a native adapter",
                chip.name
            ),
            Some(core) if core.starts_with("cortex-a") => anyhow::bail!(
                "chip '{}': unsupported ARM application core '{core}'; only cortex-a7 has an experimental native adapter",
                chip.name
            ),
            _ => Ok(MachineFamily::CortexM),
        },
        Arch::RiscV => Ok(MachineFamily::RiscV),
        Arch::Xtensa => Ok(MachineFamily::Xtensa),
        Arch::Avr => Ok(MachineFamily::Avr),
        Arch::Pic24 => Ok(MachineFamily::Pic24),
        Arch::Pic18 => Ok(MachineFamily::Pic18),
        Arch::Pic16 => Ok(MachineFamily::Pic16),
        Arch::Rx => Ok(MachineFamily::Rx),
        Arch::Aarch64 => Ok(MachineFamily::Aarch64),
        Arch::Unknown => anyhow::bail!(
            "chip '{}' does not declare a known architecture (`arch:` must be arm, riscv, xtensa, avr, pic24, pic18, pic16, rx, or aarch64)",
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
        // EM_DSPIC30F (binutils elf.h) — the machine Microchip's dsPIC30/33
        // toolchains stamp on 16-bit MCU/DSC ELFs.
        118 => Some(crate::Arch::Pic24),
        // EM_MCHP_PIC (204), the registered ELF machine value for Microchip's
        // 8-bit PIC family (gABI). It is the only 8-bit PIC machine value
        // available, so it names PIC18 here and the chip retargets a PIC16
        // image ([`image_arch_for_chip`]).
        204 => Some(crate::Arch::Pic18),
        goblin::elf::header::EM_AARCH64 => Some(crate::Arch::Aarch64),
        // EM_RX (173), Renesas RX (gABI; binutils elf/common.h).
        173 => Some(crate::Arch::Rx),
        _ => None,
    }
}

/// The architecture an image runs as on `chip_arch`.
///
/// EM_MCHP_PIC covers both 8-bit PIC families (PIC16 and PIC18 share it and
/// the ELF header does not tell them apart), so [`elf_arch`] reports such an
/// image as PIC18. When the chip declares `arch: pic16`, the chip is the one
/// fact that says which core the bytes are for. Every other pairing is
/// returned unchanged, so a real mismatch (an ARM image on a PIC chip) is
/// still refused by the caller.
pub fn image_arch_for_chip(image_arch: crate::Arch, chip_arch: Arch) -> crate::Arch {
    match (image_arch, chip_arch) {
        (crate::Arch::Pic18, Arch::Pic16) => crate::Arch::Pic16,
        (arch, _) => arch,
    }
}
