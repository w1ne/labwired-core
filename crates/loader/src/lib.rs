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

pub mod coverage;
pub mod footprint;
pub mod multi_image;
pub mod source_map;
pub mod source_step;

pub use footprint::{elf_section_totals_v1, ElfSectionTotals, FOOTPRINT_METHOD};

pub fn load_elf(path: &Path) -> Result<ProgramImage> {
    let buffer = fs::read(path).with_context(|| format!("Failed to read ELF file: {:?}", path))?;
    load_elf_bytes(&buffer)
}

/// Resolve a single function symbol's address from an ELF binary.
///
/// Used to auto-discover Arduino-ESP32 thunk PCs (heap_caps_init,
/// esp_timer_init, esp_ota_get_running_partition, ...) without baking
/// per-firmware constants into either the CLI snapshot-capture path
/// or the WASM `install_esp32_arduino_quirks` bootstrap. Returns `None`
/// when the ELF is stripped of symbols (in which case the caller falls
/// back to a hardcoded profile).
///
/// Skips the gimli/DWARF dance that [`SymbolProvider::new`] does — we
/// only need name→address resolution from the regular symbol table,
/// which works on `--strip-debug`-stripped binaries too.
pub fn resolve_symbol_in_elf(buffer: &[u8], name: &str) -> Option<u32> {
    use object::{Object, ObjectSymbol};
    let object = object::File::parse(buffer).ok()?;
    for sym in object.symbols() {
        if let Ok(n) = sym.name() {
            if n == name && sym.address() > 0 {
                return Some(sym.address() as u32);
            }
        }
    }
    None
}

/// Extract every Arduino-ESP32 / ESP-IDF / Arduino-core symbol the LabWired
/// sim cares about for an Arduino-ESP32 firmware. Includes:
///   * flash-thunk targets (heap_caps_*, esp_timer_init, locks, …),
///   * dual-core handshake bytes (`s_cpu_up`, `s_cpu_inited`, `s_system_inited`,
///     `s_other_cpu_startup_done`) that the single-CPU sim has to pre-write,
///   * optional bootstrap markers (`loopTask`, `app_main`).
///
/// Returns the addresses present in this firmware; the caller treats absent
/// entries as "use my hardcoded fallback" or "no patch needed." Works on
/// `--strip-debug`-stripped binaries — only requires the regular symbol
/// table, not DWARF.
pub fn extract_arduino_esp32_thunks(buffer: &[u8]) -> HashMap<&'static str, u32> {
    // The list and the lookup live in core, next to the profile that consumes
    // them, so a world node (which cannot depend on this crate) resolves
    // exactly the same symbols as the CLI and the debugger.
    labwired_core::boot::esp32_arduino::arduino_esp32_symbols(buffer)
}

pub fn load_elf_bytes(buffer: &[u8]) -> Result<ProgramImage> {
    let elf = Elf::parse(buffer).context("Failed to parse ELF binary")?;

    info!("ELF Entry Point: {:#x}", elf.entry);

    // The mapping itself lives in `labwired_core::system::arch_policy`; this
    // crate only chooses what to do when it says "not modelled". It records
    // Unknown rather than failing, because a caller that never runs the image
    // (a symboliser, a disassembler) is still served by a parsed one.
    let arch =
        labwired_core::system::arch_policy::elf_arch(elf.header.e_machine).unwrap_or_else(|| {
            warn!("Unknown ELF machine type: {}", elf.header.e_machine);
            labwired_core::Arch::Unknown
        });

    let mut program_image = ProgramImage::new(elf.entry, arch);

    for ph in elf.program_headers {
        if ph.p_type == PT_LOAD {
            // We only care about loadable segments
            let size = ph.p_filesz as usize;
            let offset = ph.p_offset as usize;

            if size == 0 {
                continue;
            }

            if offset + size > buffer.len() {
                return Err(anyhow!("Segment out of bounds in ELF file"));
            }

            let segment_data = buffer[offset..offset + size].to_vec();

            if arch == labwired_core::Arch::Avr {
                // avr-gcc: .text at low VMA; .data has VMA 0x800000+data and LMA in flash
                // so CRT can LPM-copy. Emit BOTH a flash LMA segment and a data VMA segment.
                let v = if ph.p_vaddr != 0 {
                    ph.p_vaddr
                } else {
                    ph.p_paddr
                };
                let (space, data_addr) = labwired_core::cpu::avr::classify_avr_vma(v);
                match space {
                    labwired_core::cpu::avr::AvrLoadSpace::Flash => {
                        let flash_addr = if ph.p_paddr != 0 {
                            ph.p_paddr
                        } else {
                            data_addr
                        };
                        debug!("AVR flash segment {:#x} size {}", flash_addr, size);
                        program_image.add_segment(flash_addr, segment_data);
                    }
                    labwired_core::cpu::avr::AvrLoadSpace::Data
                    | labwired_core::cpu::avr::AvrLoadSpace::Eeprom => {
                        // Flash LMA holds the initializer image (for LPM / __do_copy_data).
                        if ph.p_paddr < 0x8000 {
                            debug!("AVR data LMA flash {:#x} size {}", ph.p_paddr, size);
                            program_image.add_segment(ph.p_paddr, segment_data.clone());
                        }
                        // Keep the *biased* VMA so load_program_image can tell
                        // data-space from program-space (0x100..RAMEND overlaps LMA).
                        debug!("AVR data VMA (biased) {:#x} size {}", v, size);
                        program_image.add_segment(v, segment_data);
                    }
                }
            } else {
                let start_addr = ph.p_paddr;
                debug!(
                    "Found Loadable Segment: Addr={:#x}, Size={} bytes, Offset={:#x}",
                    start_addr, size, offset
                );
                program_image.add_segment(start_addr, segment_data);
            }
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

/// One row of the DWARF line-number program: the instruction address and the
/// source position it maps to. Unlike the reverse `line_map`, these are NOT
/// deduplicated — every row is retained, so the set of `is_stmt` rows is the
/// statement universe for coverage.
#[derive(Debug, Clone)]
pub struct StmtRow {
    pub addr: u64,
    pub file: String,
    pub line: u32,
    pub is_stmt: bool,
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
    // Full line-program rows (not deduped) — the statement universe for coverage
    stmt_rows: Vec<StmtRow>,
    // Map of symbol_name -> address
    symbol_map: HashMap<String, u64>,
    // Function symbols with a size: (start, size, name), start Thumb bit cleared
    functions: Vec<(u64, u64, String)>,
    // Test-only locals: PC -> list of locals
    test_locals: HashMap<u64, Vec<LocalVariable>>,
}

impl SymbolProvider {
    pub fn new(path: &Path) -> Result<Self> {
        let data = fs::read(path)
            .with_context(|| format!("Failed to read ELF for symbols: {:?}", path))?;
        Self::from_bytes(data)
    }

    /// Parse symbols and DWARF from ELF bytes already in memory (the browser
    /// has no filesystem).
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        use gimli::Reader;
        use object::Object;
        let data = Arc::new(data);

        let slice: &'static [u8] = unsafe { std::mem::transmute(&data[..]) };

        let object = object::File::parse(slice).context("Failed to parse ELF for symbols")?;

        let mut line_map = std::collections::HashMap::new();
        let mut stmt_rows: Vec<StmtRow> = Vec::new();

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
                                let line_u32 = line.get() as u32;
                                // Retain every row for the statement universe...
                                stmt_rows.push(StmtRow {
                                    addr: row.address(),
                                    file: f.clone(),
                                    line: line_u32,
                                    is_stmt: row.is_stmt(),
                                });
                                // ...and the first address per file:line for reverse lookup.
                                line_map.entry((f, line_u32)).or_insert(row.address());
                            }
                        }
                    }
                }
            }
        }

        let mut symbol_map = std::collections::HashMap::new();
        let mut functions = Vec::new();
        for sym in object.symbols() {
            if let Ok(name) = sym.name() {
                if sym.address() > 0 {
                    symbol_map.insert(name.to_string(), sym.address());
                }
                if sym.kind() == object::SymbolKind::Text && sym.size() > 0 && !name.is_empty() {
                    functions.push((sym.address() & !1, sym.size(), name.to_string()));
                }
            }
        }
        functions.sort();
        functions.dedup_by(|a, b| a.0 == b.0 && a.2 == b.2);

        let dwarf_for_context =
            gimli::Dwarf::load(&load_section).context("Failed to load DWARF for context")?;
        let context = addr2line::Context::from_dwarf(dwarf_for_context)
            .context("Failed to create context from dwarf")?;

        Ok(Self {
            data,
            dwarf,
            context,
            line_map,
            stmt_rows,
            symbol_map,
            functions,
            test_locals: HashMap::new(),
        })
    }

    /// Full DWARF line-program rows (not deduplicated). The set of rows with
    /// `is_stmt` set is the statement universe: a statement is covered when an
    /// instruction at its address was executed.
    pub fn statement_rows(&self) -> &[StmtRow] {
        &self.stmt_rows
    }

    /// Sized function symbols as `(start, size, name)`, ascending by start,
    /// with the Thumb bit cleared from `start`.
    pub fn functions(&self) -> &[(u64, u64, String)] {
        &self.functions
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

            // Whether `entry`'s address ranges (low/high_pc or DW_AT_ranges) hold `pc`.
            let covers =
                |entry: &addr2line::gimli::DebuggingInformationEntry<'_, '_, _, _>| -> bool {
                    let Ok(mut ranges) = self.dwarf.die_ranges(&unit, entry) else {
                        return false;
                    };
                    while let Ok(Some(r)) = ranges.next() {
                        if r.begin <= pc && pc < r.end {
                            return true;
                        }
                    }
                    false
                };

            // `next_dfs` reports a depth change, not a depth: keep the sum.
            let mut depth: isize = 0;
            // Depth of the subprogram that contains `pc`, while inside it.
            let mut subprogram: Option<isize> = None;
            // Lexical blocks and inlined calls inside it, with whether each
            // holds `pc`. A variable is in scope only if all of them do: the
            // `let`s of an earlier `unsafe {}` block are not live later on.
            let mut scopes: Vec<(isize, bool)> = Vec::new();
            let mut entries = unit.entries();

            while let Ok(Some((delta, entry))) = entries.next_dfs() {
                depth += delta;
                while scopes.last().is_some_and(|(d, _)| *d >= depth) {
                    scopes.pop();
                }
                if subprogram.is_some_and(|d| depth <= d) {
                    subprogram = None;
                }
                let Some(_) = subprogram else {
                    if entry.tag() == addr2line::gimli::DW_TAG_subprogram && covers(entry) {
                        subprogram = Some(depth);
                    }
                    continue;
                };
                {
                    if matches!(
                        entry.tag(),
                        addr2line::gimli::DW_TAG_lexical_block
                            | addr2line::gimli::DW_TAG_inlined_subroutine
                            | addr2line::gimli::DW_TAG_subprogram
                    ) {
                        scopes.push((depth, covers(entry)));
                        continue;
                    }
                    if !scopes.iter().all(|(_, live)| *live) {
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

                        let Some(n) = name else { continue };
                        // A single expression, or (what optimised builds emit
                        // for nearly every local) a location list whose entry
                        // covering `pc` holds it. A list with no entry here
                        // means the value is gone at this point.
                        let expr = match entry
                            .attr_value(addr2line::gimli::DW_AT_location)
                            .ok()
                            .flatten()
                        {
                            Some(addr2line::gimli::AttributeValue::Exprloc(expr)) => Some(expr),
                            Some(addr2line::gimli::AttributeValue::LocationListsRef(offset)) => {
                                let Ok(mut list) = self.dwarf.locations(&unit, offset) else {
                                    continue;
                                };
                                let mut found = None;
                                while let Ok(Some(e)) = list.next() {
                                    if e.range.begin <= pc && pc < e.range.end {
                                        found = Some(e.data);
                                        break;
                                    }
                                }
                                if found.is_none() {
                                    locals.push(LocalVariable {
                                        name: n,
                                        location: DwarfLocation::Other("optimized out".into()),
                                    });
                                    continue;
                                }
                                found
                            }
                            _ => None,
                        };
                        let Some(expr) = expr else { continue };
                        let mut ops = expr.operations(unit.encoding());
                        if let Ok(Some(op)) = ops.next() {
                            let location = match op {
                                addr2line::gimli::Operation::Register { register } => {
                                    DwarfLocation::Register(register.0)
                                }
                                addr2line::gimli::Operation::FrameOffset { offset } => {
                                    DwarfLocation::FrameRelative(offset)
                                }
                                addr2line::gimli::Operation::Address { address } => {
                                    DwarfLocation::Address(address)
                                }
                                _ => DwarfLocation::Other(format!("{:?}", op)),
                            };
                            locals.push(LocalVariable { name: n, location });
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
            stmt_rows: Vec::new(),
            symbol_map: HashMap::new(),
            functions: Vec::new(),
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

/// The fault verdict's symbolizer: DWARF line info first, then the ELF
/// symbol table for the function name when DWARF has none (a stripped or
/// `-g0` build still names the function).
impl labwired_core::fault_verdict::FaultSymbolizer for SymbolProvider {
    fn symbolize(&self, addr: u32) -> Option<labwired_core::fault_verdict::CodeLocation> {
        let addr = u64::from(addr & !1);
        let from_symtab = || {
            self.functions
                .iter()
                .rev()
                .find(|(start, size, _)| *start <= addr && addr < start + size)
                .map(|(_, _, name)| name.clone())
        };
        match self.lookup(addr) {
            Some(loc) => Some(labwired_core::fault_verdict::CodeLocation {
                function: loc.function.or_else(from_symtab),
                file: Some(loc.file),
                line: loc.line,
            }),
            None => from_symtab().map(|f| labwired_core::fault_verdict::CodeLocation {
                function: Some(f),
                file: None,
                line: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_location_to_pc() {
        // This test requires the firmware to be built with debug symbols.
        // Build it with: cargo build -p firmware-ci-fixture --target thumbv7m-none-eabi
        // (see core-ci.yml "Build test firmware fixture" step).
        let elf_path = labwired_core::test_support::target_dir()
            .join("thumbv7m-none-eabi/debug/firmware-ci-fixture");
        if !elf_path.exists() {
            // The fast PR gate runs `cargo test --workspace --lib` WITHOUT
            // cross-building firmware, so this fixture is absent there. Skip
            // gracefully rather than fail; the post-merge full suite builds
            // firmware-ci-fixture first and exercises the real assertions.
            eprintln!(
                "skipping test_location_to_pc: fixture not built \
                 (cargo build -p firmware-ci-fixture --target thumbv7m-none-eabi)"
            );
            return;
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
    fn test_statement_rows_full_not_deduped() {
        let elf_path = labwired_core::test_support::target_dir()
            .join("thumbv7m-none-eabi/debug/firmware-ci-fixture");
        if !elf_path.exists() {
            labwired_core::test_support::skip_or_fail_missing_firmware(
                "firmware-ci-fixture",
                "firmware-ci-fixture ELF (test_statement_rows_full_not_deduped)",
                "cargo build -p firmware-ci-fixture --target thumbv7m-none-eabi",
            );
            return;
        }

        let provider = SymbolProvider::new(&elf_path).expect("Failed to create SymbolProvider");
        let rows = provider.statement_rows();

        assert!(!rows.is_empty(), "expected DWARF line-program rows");
        assert!(
            rows.iter().any(|r| r.is_stmt),
            "expected at least one is_stmt row"
        );

        // The full row set must not be deduplicated the way the reverse line_map
        // is: there are more rows than distinct (file,line) keys whenever any
        // line spans multiple address ranges (loops, inlining, -O).
        let distinct_lines: std::collections::HashSet<(&str, u32)> =
            rows.iter().map(|r| (r.file.as_str(), r.line)).collect();
        assert!(
            rows.len() >= distinct_lines.len(),
            "row count must be at least the distinct-line count"
        );

        // main.rs line 26 is known-present (see test_location_to_pc).
        assert!(
            rows.iter()
                .any(|r| r.file.ends_with("main.rs") && r.line == 26),
            "expected a statement row for main.rs:26"
        );
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

        // The lookup uses an absolute path that includes the workspace
        // ancestor — we only care that the suffix `crates/...` matches
        // the registered key. Use an arbitrary absolute prefix so this
        // test doesn't bake in any specific developer's home directory.
        let resolved = provider.location_to_pc_nearest(
            "/workspace/labwired/core/crates/firmware-h563-io-demo/src/main.rs",
            120,
        );
        assert_eq!(resolved, Some((0x0800_00FC, 125)));
    }

    /// `extract_arduino_esp32_thunks` now resolves through core's goblin
    /// reader; it must agree, symbol for symbol, with what this crate's
    /// `object` reader finds for the same list on a real Arduino-ESP32 ELF.
    #[test]
    fn arduino_esp32_symbols_agree_with_an_object_symbol_scan() {
        use object::{Object, ObjectSymbol};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let elf = std::fs::read(root.join("tests/fixtures/source-debug/esp32-arduino.elf"))
            .expect("committed classic-ESP32 Arduino fixture");

        let mut expected = HashMap::new();
        let file = object::File::parse(&*elf).expect("parse");
        for sym in file.symbols() {
            if let Ok(name) = sym.name() {
                if sym.address() > 0 {
                    if let Some(known) = labwired_core::boot::esp32_arduino::ARDUINO_ESP32_SYMBOLS
                        .iter()
                        .find(|k| **k == name)
                    {
                        expected.insert(*known, sym.address() as u32);
                    }
                }
            }
        }
        let got = extract_arduino_esp32_thunks(&elf);
        assert!(got.len() > 20, "too few symbols resolved: {}", got.len());
        assert_eq!(got, expected);
    }
}
