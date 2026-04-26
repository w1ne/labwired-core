// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Fast-boot for ESP32-S3 ELFs (`xtensa-esp32s3-none-elf` target).
//!
//! Skips the BROM and 2nd-stage bootloader; places ELF segments at their
//! virtual addresses via the bus and synthesises post-bootloader CPU state.
//!
//! Plan 2 Task 1: this initial version places ALL segments via `bus.write_u8`.
//! Task 8 extends `BootOpts` with a `flash_backing` field and routes flash-XIP
//! segments to the shared backing buffer directly.

use crate::boot::{BootError, BootResult};
use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, Cpu};
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;

/// Per-call options for `fast_boot`.
#[derive(Debug, Clone)]
pub struct BootOpts {
    /// Used as the SP if the ELF lacks a recognized stack-top symbol
    /// (`_stack_start_cpu0` or `_stack_top`).
    pub stack_top_fallback: u32,
}

/// Result of a successful boot.
#[derive(Debug, Clone, Copy)]
pub struct BootSummary {
    pub entry: u32,
    pub stack: u32,
    pub segments_loaded: usize,
}

/// Load `elf_bytes` into the bus, set the CPU's PC and SP, return a summary.
///
/// This function does NOT touch peripherals other than via `Bus::write_u8`.
/// The caller is responsible for having registered all relevant peripherals
/// (IRAM, DRAM, etc.) before calling.
pub fn fast_boot(
    elf_bytes: &[u8],
    bus: &mut SystemBus,
    cpu: &mut XtensaLx7,
    opts: &BootOpts,
) -> BootResult<BootSummary> {
    let elf = Elf::parse(elf_bytes).map_err(|e| BootError::ElfParse(format!("{e}")))?;

    let mut segments_loaded = 0;
    for ph in &elf.program_headers {
        // Skip BSS-only segments (p_filesz == 0); they're zero-init from
        // the bus's RAM regions. Only load segments with file content.
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let vaddr = ph.p_vaddr as u32;
        let file_off = ph.p_offset as usize;
        let size = ph.p_filesz as usize;
        let end = file_off.checked_add(size).ok_or_else(|| {
            BootError::ElfParse(format!(
                "segment p_offset 0x{file_off:x} + p_filesz 0x{size:x} overflows usize"
            ))
        })?;
        let bytes = elf_bytes.get(file_off..end).ok_or_else(|| {
            BootError::ElfParse(format!(
                "segment beyond file: offset 0x{file_off:x} size 0x{size:x} file_len 0x{:x}",
                elf_bytes.len()
            ))
        })?;
        for (i, &b) in bytes.iter().enumerate() {
            let addr = vaddr.wrapping_add(i as u32) as u64;
            bus.write_u8(addr, b)
                .map_err(|_| BootError::SegmentOutsideMap {
                    addr: addr as u32,
                    size: size - i,
                })?;
        }
        segments_loaded += 1;
    }

    if segments_loaded == 0 {
        tracing::warn!(
            "fast_boot: ELF has no loadable segments — entry 0x{:08x} will likely fault",
            elf.entry
        );
    }

    // Look up `_stack_start_cpu0`; fall back to opts.stack_top_fallback.
    let stack = elf
        .syms
        .iter()
        .find(|sym| {
            let name = elf.strtab.get_at(sym.st_name).unwrap_or("");
            name == "_stack_start_cpu0" || name == "_stack_top"
        })
        .map(|sym| sym.st_value as u32)
        .unwrap_or(opts.stack_top_fallback);

    let entry = elf.entry as u32;
    cpu.set_pc(entry);
    cpu.set_sp(stack);

    Ok(BootSummary {
        entry,
        stack,
        segments_loaded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::xtensa_lx7::XtensaLx7;
    use crate::{Bus, Cpu, Peripheral, SimResult};

    /// A minimal `Peripheral` backed by a flat byte array, used to satisfy
    /// `fast_boot`'s `bus.write_u8` calls in unit tests.
    #[derive(Debug)]
    struct RamPeripheral {
        data: std::cell::RefCell<Vec<u8>>,
    }

    impl Peripheral for RamPeripheral {
        fn read(&self, offset: u64) -> SimResult<u8> {
            Ok(*self.data.borrow().get(offset as usize).unwrap_or(&0))
        }
        fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
            let mut d = self.data.borrow_mut();
            if let Some(slot) = d.get_mut(offset as usize) {
                *slot = value;
            }
            Ok(())
        }
    }

    /// Build a minimal valid Xtensa ELF in memory: one PT_LOAD segment of 4
    /// bytes at `0x4037_0000`, entry point at the same address, no symbols.
    fn build_minimal_elf() -> Vec<u8> {
        // ELF64 header (64 bytes) + 1 program header (56 bytes) + 4 bytes payload
        let mut elf = vec![0u8; 64 + 56 + 4];

        // ELF identification
        elf[0..4].copy_from_slice(b"\x7FELF");
        elf[4] = 2; // EI_CLASS = ELFCLASS64
        elf[5] = 1; // EI_DATA = ELFDATA2LSB
        elf[6] = 1; // EI_VERSION = EV_CURRENT
        elf[16] = 2; // e_type = ET_EXEC
        elf[17] = 0;
        elf[18] = 94; // e_machine = EM_XTENSA (94)
        elf[19] = 0;
        // e_version (4 bytes) at 20
        elf[20] = 1;
        // e_entry (8 bytes) at 24
        elf[24..28].copy_from_slice(&0x4037_0000u32.to_le_bytes());
        // e_phoff (8 bytes) at 32
        elf[32] = 64;
        // e_ehsize at 52, e_phentsize at 54, e_phnum at 56
        elf[52..54].copy_from_slice(&64u16.to_le_bytes());
        elf[54..56].copy_from_slice(&56u16.to_le_bytes());
        elf[56..58].copy_from_slice(&1u16.to_le_bytes());

        // Program header at offset 64
        let ph = 64;
        elf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
        elf[ph + 4..ph + 8].copy_from_slice(&5u32.to_le_bytes()); // p_flags = R+X
        elf[ph + 8..ph + 16].copy_from_slice(&120u64.to_le_bytes()); // p_offset
        elf[ph + 16..ph + 24].copy_from_slice(&0x4037_0000u64.to_le_bytes()); // p_vaddr
        elf[ph + 24..ph + 32].copy_from_slice(&0x4037_0000u64.to_le_bytes()); // p_paddr
        elf[ph + 32..ph + 40].copy_from_slice(&4u64.to_le_bytes()); // p_filesz
        elf[ph + 40..ph + 48].copy_from_slice(&4u64.to_le_bytes()); // p_memsz

        // Payload at offset 120
        elf[120..124].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);

        elf
    }

    #[test]
    fn fast_boot_places_segment_and_sets_pc_sp() {
        let elf_bytes = build_minimal_elf();

        let mut bus = SystemBus::new();
        bus.add_peripheral(
            "iram",
            0x4037_0000,
            0x1_0000,
            None,
            Box::new(RamPeripheral {
                data: std::cell::RefCell::new(vec![0u8; 0x1_0000]),
            }),
        );

        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();

        let summary = fast_boot(
            &elf_bytes,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
            },
        )
        .expect("fast_boot");

        assert_eq!(summary.entry, 0x4037_0000);
        assert_eq!(summary.stack, 0x3FCD_FFF0);
        assert_eq!(summary.segments_loaded, 1);

        assert_eq!(cpu.get_pc(), 0x4037_0000);
        assert_eq!(cpu.get_register(1), 0x3FCD_FFF0, "a1 (SP) should hold the stack top");
        assert_eq!(bus.read_u8(0x4037_0000).unwrap(), 0xAA);
        assert_eq!(bus.read_u8(0x4037_0003).unwrap(), 0xDD);
    }

    #[test]
    fn fast_boot_reports_segment_outside_map_when_no_peripheral_covers_vaddr() {
        // SystemBus is strict: writes to addresses outside any peripheral
        // return MemoryViolation. fast_boot maps that to SegmentOutsideMap
        // so the caller knows their peripheral map is missing a region the
        // ELF wants to load into.
        let elf_bytes = build_minimal_elf();

        let mut bus = SystemBus::new();
        // Note: no IRAM peripheral mapped at 0x4037_0000.
        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus).unwrap();

        let res = fast_boot(
            &elf_bytes,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
            },
        );

        match res {
            Err(BootError::SegmentOutsideMap { addr, size }) => {
                assert_eq!(addr, 0x4037_0000);
                assert_eq!(size, 4);
            }
            other => panic!("expected SegmentOutsideMap, got {other:?}"),
        }
    }
}
