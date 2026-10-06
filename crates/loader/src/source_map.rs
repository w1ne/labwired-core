// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Source-level lookups for a debugger UI: pc -> file:line, file:line -> pc,
//! the lines of a file that carry code, and the statement boundaries a
//! "step one source line" stops at.
//!
//! Built once per ELF from the DWARF line program already loaded by
//! [`SymbolProvider`]. Unlike the provider's reverse `line_map`, which is keyed
//! by the bare file name, every row here carries the file's full path
//! (`comp_dir` / include directory / name), so `lib.rs` of one crate is not
//! confused with `lib.rs` of another and an editor path can be matched by its
//! trailing components.
//!
//! Line-program sequences whose first address is not inside an executable
//! section are dropped. Linkers leave the line rows of garbage-collected
//! functions in place with their address tombstoned to 0 (or -1), and those
//! would otherwise answer "line 12 is at address 0".

use crate::SymbolProvider;
use addr2line::gimli;
use addr2line::gimli::Reader;
use std::collections::HashSet;

/// Marks a row that ends a line-program sequence: addresses from here on have
/// no source position until the next sequence starts.
const END_OF_SEQUENCE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy)]
struct Row {
    addr: u64,
    /// Index into [`SourceMap::files`], or [`END_OF_SEQUENCE`].
    file: u32,
    line: u32,
    /// 0 when the producer recorded no column.
    column: u32,
    is_stmt: bool,
}

/// A source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePos {
    /// The full path DWARF records (comp_dir joined with the include
    /// directory and the file name).
    pub file: String,
    pub line: u32,
    pub column: Option<u32>,
}

/// The answer to "where does line N of file F start".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineAddress {
    /// The lowest statement address on the matched line.
    pub pc: u64,
    /// Every statement address on the matched line, ascending. A line has more
    /// than one when the compiler duplicated or inlined it.
    pub pcs: Vec<u64>,
    /// The DWARF path of the file that matched.
    pub file: String,
    /// The line actually matched; differs from the request when that line has
    /// no code.
    pub line: u32,
}

/// pc <-> source line index for one ELF. See the module docs.
pub struct SourceMap {
    files: Vec<String>,
    /// Sorted by address. At one address, end-of-sequence rows sort first and
    /// the rest keep line-program order, so the last row at or below a pc is
    /// the one that describes it.
    rows: Vec<Row>,
    /// Addresses that start an `is_stmt` row: where a source-level step stops.
    stmt_addrs: HashSet<u64>,
    /// Clears what a pc carries that is not part of its address: the Thumb
    /// bit on Arm. Every other architecture's pc is the address, and on
    /// Xtensa an odd one is an ordinary instruction start.
    pc_mask: u64,
}

impl SourceMap {
    /// Builds the index from the provider's DWARF.
    pub fn from_provider(symbols: &SymbolProvider) -> Self {
        let text = text_ranges(&symbols.data);
        let dwarf = &symbols.dwarf;
        let mut files: Vec<String> = Vec::new();
        let mut file_ids: std::collections::HashMap<String, u32> = Default::default();
        let mut rows: Vec<Row> = Vec::new();

        let mut units = dwarf.units();
        while let Ok(Some(header)) = units.next() {
            let Ok(unit) = dwarf.unit(header) else {
                continue;
            };
            let Some(program) = unit.line_program.clone() else {
                continue;
            };
            let comp_dir = unit
                .comp_dir
                .as_ref()
                .and_then(|d| d.to_string_lossy().ok().map(|s| s.into_owned()));

            let mut sequence: Vec<Row> = Vec::new();
            let mut iter = program.rows();
            while let Ok(Some((line_header, row))) = iter.next_row() {
                if row.end_sequence() {
                    let keep = sequence
                        .first()
                        .is_some_and(|first| text.iter().any(|r| r.contains(&first.addr)));
                    if keep {
                        rows.append(&mut sequence);
                        rows.push(Row {
                            addr: row.address(),
                            file: END_OF_SEQUENCE,
                            line: 0,
                            column: 0,
                            is_stmt: false,
                        });
                    }
                    sequence.clear();
                    continue;
                }
                let Some(line) = row.line() else {
                    continue;
                };
                let Some(entry) = line_header.file(row.file_index()) else {
                    continue;
                };
                let name = attr_text(dwarf, &unit, entry.path_name());
                let dir = entry
                    .directory(line_header)
                    .and_then(|d| attr_text(dwarf, &unit, d));
                let Some(name) = name else {
                    continue;
                };
                let path = join_path(comp_dir.as_deref(), dir.as_deref(), &name);
                let next = files.len() as u32;
                let file = *file_ids.entry(path.clone()).or_insert_with(|| {
                    files.push(path);
                    next
                });
                let column = match row.column() {
                    gimli::ColumnType::LeftEdge => 0,
                    gimli::ColumnType::Column(c) => u32::try_from(c.get()).unwrap_or(0),
                };
                sequence.push(Row {
                    addr: row.address(),
                    file,
                    line: u32::try_from(line.get()).unwrap_or(u32::MAX - 1),
                    column,
                    is_stmt: row.is_stmt(),
                });
            }
        }

        // Stable: rows at one address keep line-program order.
        rows.sort_by_key(|r| (r.addr, r.file != END_OF_SEQUENCE));
        let stmt_addrs = rows
            .iter()
            .filter(|r| r.is_stmt && r.file != END_OF_SEQUENCE)
            .map(|r| r.addr)
            .collect();
        // EM_ARM; e_machine is at byte 18 of the ELF header.
        let arm = symbols.data.get(18..20) == Some(&[40, 0][..]);
        Self {
            files,
            rows,
            stmt_addrs,
            pc_mask: if arm { !1 } else { !0 },
        }
    }

    /// `pc` as a code address: without the Arm Thumb bit, else unchanged.
    pub fn code_address(&self, pc: u64) -> u64 {
        pc & self.pc_mask
    }

    /// True when the ELF carries no usable line information.
    pub fn is_empty(&self) -> bool {
        self.stmt_addrs.is_empty()
    }

    /// The source position of the instruction at `pc` (Thumb bit ignored).
    pub fn location(&self, pc: u64) -> Option<SourcePos> {
        let row = self.row_at(pc & self.pc_mask)?;
        Some(SourcePos {
            file: self.files[row.file as usize].clone(),
            line: row.line,
            column: (row.column != 0).then_some(row.column),
        })
    }

    /// [`Self::location`] for `pc` known to be a function's first
    /// instruction.
    ///
    /// gcc gives the entry address the opening line, then the first
    /// statement's line, then the opening line again as a non-statement row
    /// that covers the prologue's register saves. `location` reports that
    /// last row, the opening line; a breakpoint set on the first statement
    /// lands on this address, so here the statement row wins. Only when the
    /// last row restates the first row's line: an entry whose last row names
    /// something else (an inlined callee's body) keeps it.
    pub fn location_at_entry(&self, pc: u64) -> Option<SourcePos> {
        let pc = pc & self.pc_mask;
        let last = self.row_at(pc)?;
        let start = self.rows.partition_point(|r| r.addr < pc);
        let end = self.rows.partition_point(|r| r.addr <= pc);
        // Rows at exactly `pc`, end-of-sequence markers (sorted first) skipped.
        let at: Vec<&Row> = self.rows[start..end]
            .iter()
            .filter(|r| r.file != END_OF_SEQUENCE)
            .collect();
        let row = at
            .first()
            .filter(|f| (f.file, f.line) == (last.file, last.line))
            .and_then(|f| {
                at[1..]
                    .iter()
                    .find(|r| r.is_stmt && (r.file, r.line) != (f.file, f.line))
            })
            .copied()
            .unwrap_or(last);
        Some(SourcePos {
            file: self.files[row.file as usize].clone(),
            line: row.line,
            column: (row.column != 0).then_some(row.column),
        })
    }

    /// `(file index, line)` of `pc`, for cheap comparisons while stepping.
    pub fn line_key(&self, pc: u64) -> Option<(u32, u32)> {
        self.row_at(pc & self.pc_mask).map(|r| (r.file, r.line))
    }

    /// Whether `pc` starts a statement (an `is_stmt` row begins there).
    pub fn is_statement(&self, pc: u64) -> bool {
        self.stmt_addrs.contains(&(pc & self.pc_mask))
    }

    /// The statement address for `line` of the file best matching `file`.
    ///
    /// File matching: paths are compared component by component from the end
    /// (`\` and `/` both separate; `.` components are ignored), and a DWARF
    /// file scores the number of trailing components it shares with `file`.
    /// The highest score wins and must be at least 1 (the file name itself),
    /// so `src/main.rs` and `stm32f103-blinky/src/main.rs` both find
    /// `/build/demo-blinky/src/main.rs`, scoring 2. DWARF files tied on the
    /// best score are searched together.
    ///
    /// Line choice (the same order as
    /// [`SymbolProvider::location_to_pc_nearest`]): the requested line if it
    /// has a statement, else the nearest following line that has one, else
    /// the nearest preceding one. On the chosen line the lowest address wins
    /// (`pc`); all of them are in `pcs`.
    pub fn line_to_pc(&self, file: &str, line: u32) -> Option<LineAddress> {
        let matched = self.matching_files(file);
        if matched.is_empty() {
            return None;
        }
        let stmts = || {
            self.rows
                .iter()
                .filter(|r| r.is_stmt && r.file != END_OF_SEQUENCE && matched.contains(&r.file))
        };
        let chosen = stmts()
            .map(|r| r.line)
            .filter(|l| *l >= line)
            .min()
            .or_else(|| stmts().map(|r| r.line).filter(|l| *l < line).max())?;
        let mut on_line: Vec<&Row> = stmts().filter(|r| r.line == chosen).collect();
        on_line.sort_by_key(|r| r.addr);
        let first = on_line.first()?;
        let mut pcs: Vec<u64> = on_line.iter().map(|r| r.addr).collect();
        pcs.dedup();
        Some(LineAddress {
            pc: first.addr,
            pcs,
            file: self.files[first.file as usize].clone(),
            line: chosen,
        })
    }

    /// The lines of the file best matching `file` that carry a statement,
    /// ascending (matching as in [`Self::line_to_pc`]).
    pub fn lines(&self, file: &str) -> Vec<u32> {
        let matched = self.matching_files(file);
        let mut lines: Vec<u32> = self
            .rows
            .iter()
            .filter(|r| r.is_stmt && r.file != END_OF_SEQUENCE && matched.contains(&r.file))
            .map(|r| r.line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    /// Every file the line program names, full paths.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    fn row_at(&self, pc: u64) -> Option<&Row> {
        let after = self.rows.partition_point(|r| r.addr <= pc);
        let row = self.rows.get(after.checked_sub(1)?)?;
        (row.file != END_OF_SEQUENCE).then_some(row)
    }

    fn matching_files(&self, file: &str) -> Vec<u32> {
        let wanted = components(file);
        let scores: Vec<usize> = self
            .files
            .iter()
            .map(|f| shared_suffix(&wanted, &components(f)))
            .collect();
        let best = scores.iter().copied().max().unwrap_or(0);
        if best == 0 {
            return Vec::new();
        }
        scores
            .iter()
            .enumerate()
            .filter(|(_, s)| **s == best)
            .map(|(i, _)| i as u32)
            .collect()
    }
}

fn components(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != ".")
        .collect()
}

fn shared_suffix(a: &[&str], b: &[&str]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

fn is_absolute(p: &str) -> bool {
    p.starts_with('/') || p.starts_with('\\') || p.get(1..2) == Some(":")
}

fn join_path(comp_dir: Option<&str>, dir: Option<&str>, name: &str) -> String {
    if is_absolute(name) {
        return name.to_string();
    }
    let mut out = String::new();
    let dir = dir.filter(|d| !d.is_empty());
    if dir.is_none_or(|d| !is_absolute(d)) {
        if let Some(c) = comp_dir.filter(|c| !c.is_empty()) {
            out.push_str(c);
        }
    }
    for part in dir.into_iter().chain(std::iter::once(name)) {
        if !out.is_empty() && !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(part);
    }
    out
}

type DwarfReader =
    addr2line::gimli::EndianReader<addr2line::gimli::RunTimeEndian, std::sync::Arc<[u8]>>;

fn attr_text(
    dwarf: &gimli::Dwarf<DwarfReader>,
    unit: &gimli::Unit<DwarfReader>,
    attr: gimli::AttributeValue<DwarfReader>,
) -> Option<String> {
    let s = dwarf.attr_string(unit, attr).ok()?;
    s.to_string_lossy().ok().map(|c| c.into_owned())
}

/// Address ranges of the ELF's executable sections.
fn text_ranges(elf: &[u8]) -> Vec<std::ops::Range<u64>> {
    use object::{Object, ObjectSection, SectionKind};
    let Ok(file) = object::File::parse(elf) else {
        return Vec::new();
    };
    file.sections()
        .filter(|s| s.kind() == SectionKind::Text && s.size() > 0)
        .map(|s| s.address()..s.address() + s.size())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_scores_count_trailing_components() {
        let dwarf = components("/build/examples/demo-blinky/src/main.rs");
        assert_eq!(shared_suffix(&components("src/main.rs"), &dwarf), 2);
        assert_eq!(
            shared_suffix(&components("stm32f103-blinky/src/main.rs"), &dwarf),
            2
        );
        assert_eq!(shared_suffix(&components("main.rs"), &dwarf), 1);
        assert_eq!(shared_suffix(&components("lib.rs"), &dwarf), 0);
        assert_eq!(
            shared_suffix(&components(".\\src\\main.rs"), &dwarf),
            2,
            "backslashes and `.` are normalised"
        );
    }

    fn fixture(name: &str) -> SourceMap {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name);
        let symbols = SymbolProvider::from_bytes(std::fs::read(path).unwrap()).unwrap();
        SourceMap::from_provider(&symbols)
    }

    /// The committed nRF54L15 smart-ring probe (gcc -Os -g3, Cortex-M33).
    /// Addresses are from `arm-none-eabi-objdump -dl` of the fixture.
    #[test]
    fn smart_ring_pc_to_line_and_back() {
        let map = fixture("nrf54l15-smart-ring.elf");
        assert!(!map.is_empty());

        // 0x2ca: `mov r3, r4` opening `probe8("imu ...")` on main.c:223.
        let at = map.location(0x2ca).unwrap();
        assert!(at.file.ends_with("/src/main.c"), "{}", at.file);
        assert!(at.file.starts_with('/'), "full path, got {}", at.file);
        assert_eq!(at.line, 223);
        // Mid-line instruction (the `bl probe8`) is still line 223.
        assert_eq!(map.location(0x2d2).unwrap().line, 223);
        // Thumb bit is ignored.
        assert_eq!(map.location(0x2cb).unwrap().line, 223);

        for editor_path in ["src/main.c", "main.c", "smart-ring-project/src/main.c"] {
            let hit = map.line_to_pc(editor_path, 223).unwrap();
            assert_eq!(hit.pc, 0x2ca, "{editor_path}");
            assert_eq!(hit.line, 223);
            assert_eq!(hit.file, at.file);
        }
        // Round trip on every line main() owns.
        for line in [219, 220, 223, 224, 228, 230] {
            let hit = map.line_to_pc("src/main.c", line).unwrap();
            assert_eq!(map.location(hit.pc).unwrap().line, line, "line {line}");
            assert!(map.is_statement(hit.pc));
        }
        // probe8's entry (0x1d8) carries 176 (`{`), 177 and 176 again
        // (non-statement): at a function entry the statement row wins, so
        // the breakpoint address for 177 reports 177.
        assert_eq!(map.line_to_pc("src/main.c", 177).unwrap().pc, 0x1d8);
        assert_eq!(map.location(0x1d8).unwrap().line, 176);
        assert_eq!(map.location_at_entry(0x1d9).unwrap().line, 177);
        // uart_puts' entry (0x160) ends on the inlined str_len (45): kept.
        assert_eq!(map.location_at_entry(0x160).unwrap().line, 45);
        // A comment line snaps to the next line with code.
        let snapped = map.line_to_pc("src/main.c", 226).unwrap();
        assert_eq!((snapped.line, snapped.pc), (228, 0x382));
        // Unknown files match nothing.
        assert!(map.line_to_pc("src/other.c", 10).is_none());
        assert!(map.location(0xFFFF_0000).is_none());
    }

    #[test]
    fn smart_ring_lines_lists_only_lines_with_code() {
        let map = fixture("nrf54l15-smart-ring.elf");
        let lines = map.lines("src/main.c");
        for l in [219, 220, 223, 224, 228, 230] {
            assert!(lines.contains(&l), "{l} missing from {lines:?}");
        }
        for l in [221, 222, 226, 227] {
            assert!(!lines.contains(&l), "{l} has no code");
        }
        assert!(lines.windows(2).all(|w| w[0] < w[1]));
        assert!(!map.lines("startup.c").is_empty());
    }

    /// Rust, RISC-V, DWARF 4 with a directory relative to comp_dir.
    #[test]
    fn riscv_fixture_joins_relative_directories() {
        let map = fixture("riscv-ci-fixture.elf");
        let main = map
            .files()
            .iter()
            .find(|f| f.ends_with("riscv-ci-fixture/src/main.rs"))
            .expect("main.rs in the line program");
        assert!(main.starts_with('/'), "comp_dir joined: {main}");
        let hit = map
            .line_to_pc("labwired/core/crates/riscv-ci-fixture/src/main.rs", 12)
            .unwrap();
        assert_eq!(hit.pc, 0x8000_02ec);
        assert_eq!(map.location(0x8000_02ec).unwrap().line, 12);
        assert_eq!(map.line_to_pc("src/main.rs", 24).unwrap().pc, 0x8000_0320);
    }

    #[test]
    fn join_path_follows_dwarf_rules() {
        assert_eq!(join_path(Some("/c"), Some("src"), "m.c"), "/c/src/m.c");
        assert_eq!(join_path(Some("/c"), Some("/abs"), "m.c"), "/abs/m.c");
        assert_eq!(join_path(Some("/c"), None, "m.c"), "/c/m.c");
        assert_eq!(join_path(Some("/c"), Some("src"), "/x/m.c"), "/x/m.c");
        assert_eq!(join_path(None, Some("src"), "m.c"), "src/m.c");
    }
}
