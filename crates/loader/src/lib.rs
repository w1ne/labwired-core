// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use anyhow::{anyhow, Context, Result};
use goblin::elf::program_header::PT_LOAD;
use goblin::elf::Elf;
use labwired_core::memory::ProgramImage;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub fn load_elf(path: &Path) -> Result<ProgramImage> {
    let buffer = fs::read(path).with_context(|| format!("Failed to read ELF file: {:?}", path))?;

    let elf = Elf::parse(&buffer).context("Failed to parse ELF binary")?;

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

pub struct SymbolProvider {
    #[allow(dead_code)]
    data: Arc<Vec<u8>>,
    context: addr2line::Context<
        addr2line::gimli::EndianReader<addr2line::gimli::RunTimeEndian, Arc<[u8]>>,
    >,
    // Map of (file_name, line) -> address
    line_map: std::collections::HashMap<(String, u32), u64>,
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

        let context =
            addr2line::Context::from_dwarf(dwarf).context("Failed to create context from dwarf")?;

        Ok(Self {
            data,
            context,
            line_map,
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
        let requested_path = std::path::Path::new(file_path);
        let requested_file = requested_path.file_name()?.to_str()?;

        // Try exact match first
        if let Some(addr) = self.line_map.get(&(file_path.to_string(), line)) {
            return Some(*addr);
        }

        // Try base name match if full path doesn't match
        for ((f, l), addr) in &self.line_map {
            if *l == line {
                // Normalize paths: check if requested path is a suffix of the stored path or vice versa
                let current_path = std::path::Path::new(f);
                if let Some(current_file) = current_path.file_name().and_then(|n| n.to_str()) {
                    if current_file == requested_file {
                        return Some(*addr);
                    }
                }
            }
        }

        None
    }
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
        assert!(loc.file.ends_with("main.rs"), "Resolved file '{}' does not end with 'main.rs'", loc.file);
        assert_eq!(loc.line, Some(26));
    }
}
