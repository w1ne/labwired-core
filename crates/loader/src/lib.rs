// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use addr2line::gimli::Reader;
use anyhow::{anyhow, Context, Result};
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;
use labwired_core::memory::ProgramImage;
use object::ObjectSymbol;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub fn load_elf(path: &Path) -> Result<ProgramImage> {
    let buffer = fs::read(path).with_context(|| format!("Failed to read ELF file: {:?}", path))?;
    load_elf_bytes(&buffer)
}

pub fn load_elf_bytes(buffer: &[u8]) -> Result<ProgramImage> {
    let elf = Elf::parse(buffer).context("Failed to parse ELF binary")?;

    info!("ELF Entry Point: {:#x}", elf.entry);

    let arch = match elf.header.e_machine {
        goblin::elf::header::EM_ARM => labwired_core::Arch::Arm,
        goblin::elf::header::EM_RISCV => labwired_core::Arch::RiscV,
        _ => {
            warn!("Unknown ELF machine type: {}", elf.header.e_machine);
            labwired_core::Arch::Unknown
        }
    };

    let mut program_image = ProgramImage::new(elf.entry, arch);

    for ph in elf.program_headers {
        if ph.p_type == PT_LOAD {
            // We only care about loadable segments
            let start_addr = ph.p_paddr; // Physical address (LMA) is usually what we want for flash programming
            let size = ph.p_filesz as usize;
            let offset = ph.p_offset as usize;

            if size == 0 {
                continue;
            }

            debug!(
                "Found Loadable Segment: Addr={:#x}, Size={} bytes, Offset={:#x}",
                start_addr, size, offset
            );

            if offset + size > buffer.len() {
                return Err(anyhow!("Segment out of bounds in ELF file"));
            }

            let segment_data = buffer[offset..offset + size].to_vec();
            program_image.add_segment(start_addr, segment_data);
        }
    }

    if program_image.segments.is_empty() {
        warn!("No loadable segments found in ELF file");
    }

    Ok(program_image)
}

pub struct SourceLocation {
    pub file: String,
    pub line: Option<u32>,
    pub function: Option<String>,
}

#[derive(Debug, Clone)]
pub enum DwarfLocation {
    Register(u16),
    Address(u64),
    FrameRelative(i64),
    Other(String),
}

#[derive(Debug, Clone)]
pub struct LocalVariable {
    pub name: String,
    pub location: DwarfLocation,
}

pub struct SymbolProvider {
    #[allow(dead_code)]
    data: Arc<Vec<u8>>,
    dwarf: addr2line::gimli::Dwarf<
        addr2line::gimli::EndianReader<addr2line::gimli::RunTimeEndian, Arc<[u8]>>,
    >,
    context: addr2line::Context<
        addr2line::gimli::EndianReader<addr2line::gimli::RunTimeEndian, Arc<[u8]>>,
    >,
    // Map of (file_name, line) -> address
    line_map: HashMap<(String, u32), u64>,
    // Map of symbol_name -> address
    symbol_map: HashMap<String, u64>,
    // Test-only locals: PC -> list of locals
    test_locals: HashMap<u64, Vec<LocalVariable>>,
}

impl SymbolProvider {
    pub fn new(path: &Path) -> Result<Self> {
        use gimli::Reader;
        use object::Object;
        let data = fs::read(path)
            .with_context(|| format!("Failed to read ELF for symbols: {:?}", path))?;
        let data = Arc::new(data);

        let slice: &'static [u8] = unsafe { std::mem::transmute(&data[..]) };

        let object = object::File::parse(slice).context("Failed to parse ELF for symbols")?;

        let mut line_map = std::collections::HashMap::new();

        // Build line map using gimli for reverse lookup
        let load_section = |id: gimli::SectionId| -> std::result::Result<
            addr2line::gimli::EndianReader<gimli::RunTimeEndian, Arc<[u8]>>,
            gimli::Error,
        > {
            use object::ObjectSection;
            let data = object
                .section_by_name(id.name())
                .and_then(|s| s.uncompressed_data().ok())
                .map(|d| Arc::from(&d[..]))
                .unwrap_or_else(|| Arc::from(&[][..]));
            Ok(gimli::EndianReader::new(data, gimli::RunTimeEndian::Little))
        };

        let dwarf = gimli::Dwarf::load(&load_section).context("Failed to load DWARF")?;

        let mut iter = dwarf.units();
        while let Ok(Some(header)) = iter.next() {
            let unit = dwarf.unit(header).ok();
            if let Some(unit) = unit {
                if let Some(ref line_program) = unit.line_program {
                    let mut rows = line_program.clone().rows();
                    while let Ok(Some((_, row))) = rows.next_row() {
                        if row.end_sequence() {
                            continue;
                        }
                        let file_idx = row.file_index();
                        if let Some(file) = line_program.header().file(file_idx) {
                            let file_name = dwarf
                                .attr_string(&unit, file.path_name())
                                .ok()
                                .and_then(|s| {
                                    let s2 = s.to_string_lossy().ok()?;
                                    Some(s2.into_owned())
                                });

                            if let (Some(f), Some(line)) = (file_name, row.line()) {
                                // Store the first address seen for this file:line
                                line_map
                                    .entry((f, line.get() as u32))
                                    .or_insert(row.address());
                            }
                        }
                    }
                }
            }
        }

        let mut symbol_map = std::collections::HashMap::new();
        for sym in object.symbols() {
            if let Ok(name) = sym.name() {
                if sym.address() > 0 {
                    symbol_map.insert(name.to_string(), sym.address());
                }
            }
        }

        let dwarf_for_context =
            gimli::Dwarf::load(&load_section).context("Failed to load DWARF for context")?;
        let context = addr2line::Context::from_dwarf(dwarf_for_context)
            .context("Failed to create context from dwarf")?;

        Ok(Self {
            data,
            dwarf,
            context,
            line_map,
            symbol_map,
            test_locals: HashMap::new(),
        })
    }

    pub fn lookup(&self, addr: u64) -> Option<SourceLocation> {
        let mut frames = match self.context.find_frames(addr) {
            addr2line::LookupResult::Output(Ok(frames)) => frames,
            _ => return None,
        };

        if let Ok(Some(frame)) = frames.next() {
            let file = frame
                .location
                .as_ref()
                .and_then(|l| l.file)
                .map(|f: &str| f.to_string());
            let line = frame.location.as_ref().and_then(|l| l.line);
            let function = frame
                .function
                .as_ref()
                .and_then(|f| f.demangle().ok())
                .map(|s: std::borrow::Cow<str>| s.into_owned());

            if let Some(f) = file {
                return Some(SourceLocation {
                    file: f,
                    line,
                    function,
                });
            }
        }
        None
    }

    pub fn location_to_pc(&self, file_path: &str, line: u32) -> Option<u64> {
        self.location_to_pc_nearest(file_path, line)
            .map(|(addr, _line)| addr)
    }

    pub fn location_to_pc_nearest(&self, file_path: &str, line: u32) -> Option<(u64, u32)> {
        let requested_file = std::path::Path::new(file_path).file_name()?.to_str()?;
        let requested_norm = normalize_path_for_match(file_path);

        // Collect candidates with same basename and a path specificity score.
        let mut candidates: Vec<(u32, u64, usize)> = Vec::new();
        for ((candidate_path, candidate_line), addr) in &self.line_map {
            let Some(candidate_file) = std::path::Path::new(candidate_path)
                .file_name()
                .and_then(|n| n.to_str())
            else {
                continue;
            };
            if candidate_file != requested_file {
                continue;
            }

            let score =
                path_match_score(&requested_norm, &normalize_path_for_match(candidate_path));
            candidates.push((*candidate_line, *addr, score));
        }
        if candidates.is_empty() {
            return None;
        }

        // Prefer the most specific path match first.
        let best_score = candidates
            .iter()
            .map(|(_, _, score)| *score)
            .max()
            .unwrap_or(0);
        candidates.retain(|(_, _, score)| *score == best_score);

        // Prefer exact line, then nearest following line, then nearest previous line.
        if let Some((l, addr, _)) = candidates.iter().find(|(l, _, _)| *l == line) {
            return Some((*addr, *l));
        }

        let mut after: Vec<(u32, u64)> = candidates
            .iter()
            .filter(|(l, _, _)| *l > line)
            .map(|(l, addr, _)| (*l, *addr))
            .collect();
        after.sort_by_key(|(l, _)| *l);
        if let Some((l, addr)) = after.first() {
            return Some((*addr, *l));
        }

        let mut before: Vec<(u32, u64)> = candidates
            .iter()
            .filter(|(l, _, _)| *l < line)
            .map(|(l, addr, _)| (*l, *addr))
            .collect();
        before.sort_by_key(|(l, _)| *l);
        before.last().map(|(l, addr)| (*addr, *l))
    }

    pub fn resolve_symbol(&self, name: &str) -> Option<u64> {
        self.symbol_map.get(name).copied()
    }

    pub fn find_locals(&self, pc: u64) -> Vec<LocalVariable> {
        let mut locals = Vec::new();

        // Include test-only locals for PC 0 (default) or the specific PC
        if let Some(tl) = self.test_locals.get(&0) {
            locals.extend(tl.clone());
        }
        if pc != 0 {
            if let Some(tl) = self.test_locals.get(&pc) {
                locals.extend(tl.clone());
            }
        }

        let mut units = self.dwarf.units();

        while let Ok(Some(header)) = units.next() {
            let unit = match self.dwarf.unit(header) {
                Ok(u) => u,
                Err(_) => continue,
            };

            let mut in_subprogram = false;
            let mut subprogram_depth = 0;
            let mut entries = unit.entries();

            while let Ok(Some((depth, entry))) = entries.next_dfs() {
                if !in_subprogram {
                    if entry.tag() == addr2line::gimli::DW_TAG_subprogram {
                        let mut low_pc = None;
                        let mut high_pc = None;

                        if let Some(addr2line::gimli::AttributeValue::Addr(addr)) = entry
                            .attr_value(addr2line::gimli::DW_AT_low_pc)
                            .ok()
                            .flatten()
                        {
                            low_pc = Some(addr);
                        }

                        if let Some(attr) = entry
                            .attr_value(addr2line::gimli::DW_AT_high_pc)
                            .ok()
                            .flatten()
                        {
                            match attr {
                                addr2line::gimli::AttributeValue::Addr(addr) => {
                                    high_pc = Some(addr)
                                }
                                addr2line::gimli::AttributeValue::Udata(size) => {
                                    high_pc = low_pc.map(|l| l + size)
                                }
                                _ => {}
                            }
                        }

                        if let (Some(low), Some(high)) = (low_pc, high_pc) {
                            if pc >= low && pc < high {
                                in_subprogram = true;
                                subprogram_depth = depth;
                            }
                        }
                    }
                } else {
                    if depth <= subprogram_depth {
                        in_subprogram = false;
                        continue;
                    }

                    if entry.tag() == addr2line::gimli::DW_TAG_variable
                        || entry.tag() == addr2line::gimli::DW_TAG_formal_parameter
                    {
                        let name = entry
                            .attr_value(addr2line::gimli::DW_AT_name)
                            .ok()
                            .flatten()
                            .and_then(|attr| {
                                let s = self.dwarf.attr_string(&unit, attr).ok()?;
                                s.to_string_lossy().ok().map(|c| c.into_owned())
                            });

                        if let (Some(n), Some(addr2line::gimli::AttributeValue::Exprloc(expr))) = (
                            name,
                            entry
                                .attr_value(addr2line::gimli::DW_AT_location)
                                .ok()
                                .flatten(),
                        ) {
                            let mut ops = expr.operations(unit.encoding());
                            if let Ok(Some(op)) = ops.next() {
                                match op {
                                    addr2line::gimli::Operation::Register { register } => {
                                        locals.push(LocalVariable {
                                            name: n,
                                            location: DwarfLocation::Register(register.0),
                                        });
                                    }
                                    addr2line::gimli::Operation::FrameOffset { offset } => {
                                        locals.push(LocalVariable {
                                            name: n,
                                            location: DwarfLocation::FrameRelative(offset),
                                        });
                                    }
                                    _ => {
                                        locals.push(LocalVariable {
                                            name: n,
                                            location: DwarfLocation::Other(format!("{:?}", op)),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        locals
    }

    /// Create an empty SymbolProvider for testing
    pub fn new_empty() -> Self {
        let data = Arc::new(Vec::new());

        let load_section = |_id: gimli::SectionId| -> std::result::Result<
            addr2line::gimli::EndianReader<gimli::RunTimeEndian, Arc<[u8]>>,
            gimli::Error,
        > {
            let data = Arc::from(&[][..]);
            Ok(gimli::EndianReader::new(data, gimli::RunTimeEndian::Little))
        };

        let dwarf = gimli::Dwarf::load(&load_section).unwrap();
        let dwarf_for_context = gimli::Dwarf::load(&load_section).unwrap();
        let context = addr2line::Context::from_dwarf(dwarf_for_context).unwrap();

        Self {
            data,
            dwarf,
            context,
            line_map: HashMap::new(),
            symbol_map: HashMap::new(),
            test_locals: HashMap::new(),
        }
    }

    /// Add a mock local variable for testing
    pub fn add_test_local(&mut self, name: &str, location: DwarfLocation) {
        // We use PC 0 as the default for test locals if not specified
        self.test_locals.entry(0).or_default().push(LocalVariable {
            name: name.to_string(),
            location,
        });
    }
}

fn normalize_path_for_match(path: &str) -> String {
    path.replace('\\', "/")
}

fn path_match_score(requested_norm: &str, candidate_norm: &str) -> usize {
    if requested_norm == candidate_norm {
        return 10_000;
    }

    // Absolute IDE paths commonly end with relative DWARF paths.
    if requested_norm.ends_with(candidate_norm) {
        return 1_000 + candidate_norm.len();
    }
    if candidate_norm.ends_with(requested_norm) {
        return 900 + requested_norm.len();
    }

    // Basename-only match fallback (weak, but better than no breakpoint).
    100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_location_to_pc() {
        // This test requires the firmware to be built with debug symbols
        let elf_path = std::path::PathBuf::from("../../target/thumbv7m-none-eabi/debug/firmware");
        if !elf_path.exists() {
            return; // Skip if file not found (e.g. in some CI environments)
        }

        let provider = SymbolProvider::new(&elf_path).expect("Failed to create SymbolProvider");

        // Try to resolve a location in main.rs
        // Note: Line 14 is 'fn main() -> ! {'
        let pc = provider.location_to_pc("main.rs", 26);
        assert!(pc.is_some(), "Should resolve main.rs:26 to a PC");

        let addr = pc.unwrap();
        assert!(addr > 0, "Resolved address should be valid");

        // Reverse lookup
        let loc = provider
            .lookup(addr)
            .expect("Lookup failed for resolved PC");

        // Debug info might map to main.rs or lib core/std if inlined, but line 26 is specific enough
        println!("Resolved file: {}", loc.file);
        assert!(
            loc.file.ends_with("main.rs"),
            "Resolved file '{}' does not end with 'main.rs'",
            loc.file
        );
        assert_eq!(loc.line, Some(26));
    }

    #[test]
    fn test_location_to_pc_nearest_prefers_same_file_and_next_line() {
        let mut provider = SymbolProvider::new_empty();
        provider.line_map.insert(
            ("crates/firmware-h563-io-demo/src/main.rs".to_string(), 117),
            0x0800_00A8,
        );
        provider.line_map.insert(
            ("crates/firmware-h563-io-demo/src/main.rs".to_string(), 125),
            0x0800_00FC,
        );
        provider
            .line_map
            .insert(("main.rs".to_string(), 117), 0xDEAD_BEEF);

        let resolved = provider.location_to_pc_nearest(
            "/home/andrii/Projects/labwired/core/crates/firmware-h563-io-demo/src/main.rs",
            120,
        );
        assert_eq!(resolved, Some((0x0800_00FC, 125)));
    }
}
