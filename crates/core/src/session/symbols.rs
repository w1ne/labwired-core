// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Name → address lookup over the firmware ELF's symbol table.
//!
//! Core cannot depend on `labwired-loader` (the loader depends on core), so
//! this reads `.symtab` with goblin, the ELF parser core already uses for
//! [`crate::system::node::parse_elf_image`]. It mirrors the loader's
//! `resolve_symbol_in_elf`: the regular symbol table only (no DWARF, so a
//! `--strip-debug` image still resolves), symbols at address 0 skipped, first
//! definition of a name wins.

use goblin::elf::header::EM_ARM;
use goblin::elf::sym::STT_FUNC;
use goblin::elf::Elf;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Symbol {
    /// `st_value` as the ELF records it. For a Thumb function that includes
    /// bit 0, exactly as the vector table and a function pointer carry it.
    pub value: u64,
    /// Where the symbol's bytes live: `value` with the Thumb bit cleared for an
    /// ARM function, `value` otherwise.
    pub location: u64,
}

/// Every named, non-zero symbol in `elf`. Empty for bytes that are not an ELF
/// (a raw flash image with no companion symbols) or an ELF with no `.symtab`.
pub(crate) fn table(elf: &[u8]) -> HashMap<String, Symbol> {
    let mut out = HashMap::new();
    let Ok(parsed) = Elf::parse(elf) else {
        return out;
    };
    let arm = parsed.header.e_machine == EM_ARM;
    for sym in parsed.syms.iter() {
        if sym.st_value == 0 {
            continue;
        }
        let Some(name) = parsed.strtab.get_at(sym.st_name).filter(|n| !n.is_empty()) else {
            continue;
        };
        let location = if arm && sym.st_type() == STT_FUNC {
            sym.st_value & !1
        } else {
            sym.st_value
        };
        out.entry(name.to_string()).or_insert(Symbol {
            value: sym.st_value,
            location,
        });
    }
    out
}
