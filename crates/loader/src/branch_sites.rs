// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Static discovery of the conditional branches in a firmware ELF.
//!
//! Branch coverage needs a denominator: every conditional branch the program
//! *could* take, including the ones the tests never reach. A run only shows
//! the branches it executed, so the list comes from decoding the ELF itself.
//!
//! We walk the instructions of every sized function symbol and of every code
//! region a mapping symbol opens (`$t` / `$x`, which also reaches hand-written
//! assembly), skip the literal pools marked with `$d`, and keep every
//! conditional branch with its target and fall-through address.
//! The run's `(from, to)` edge counts then give each one a "taken" and a
//! "not taken" count.
//!
//! Supported instruction sets:
//! - **Thumb / Thumb-2** (every Cortex-M): `B<cond>` (T1 and T3), `CBZ`, `CBNZ`.
//! - **RISC-V** (RV32/RV64, with or without C): `BEQ BNE BLT BGE BLTU BGEU`,
//!   `C.BEQZ`, `C.BNEZ`.
//!
//! Other ISAs return `isa: None`, and the report says branch coverage is not
//! available rather than printing a wrong number.
//!
//! Known blind spot (shared by every object-code coverage tool): a source
//! decision the compiler turns into predicated code (a Thumb `IT` block, a
//! conditional select) has no branch instruction, so it is not listed. Build
//! with `-O0`/`-Og` when branch numbers matter.

use object::{Object, ObjectSection, ObjectSymbol, SectionKind, SymbolKind};
use serde::Serialize;

/// Instruction set the branches were decoded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BranchIsa {
    Thumb,
    RiscV,
}

impl BranchIsa {
    pub fn as_str(self) -> &'static str {
        match self {
            BranchIsa::Thumb => "thumb",
            BranchIsa::RiscV => "riscv",
        }
    }
}

/// One conditional branch instruction found in the ELF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StaticBranch {
    /// Address of the branch instruction (Thumb bit cleared).
    pub addr: u64,
    /// Encoded length in bytes (2 or 4).
    pub len: u8,
    /// Where execution goes when the condition holds.
    pub target: u64,
}

impl StaticBranch {
    /// Where execution goes when the condition fails.
    pub fn fallthrough(&self) -> u64 {
        self.addr + u64::from(self.len)
    }
}

/// All conditional branches of one ELF, ascending by address.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchSites {
    /// `None` when the ELF's machine type has no decoder here.
    pub isa: Option<BranchIsa>,
    pub sites: Vec<StaticBranch>,
}

/// Decode every conditional branch in `elf`. Never fails: an ELF that can't be
/// parsed, or an ISA without a decoder, gives an empty result with `isa: None`.
pub fn decode(elf: &[u8]) -> BranchSites {
    decode_with_code_ranges(elf, &[])
}

/// [`decode`], also walking `code_ranges`: `[start, end)` address ranges
/// known to hold instructions and to start on an instruction boundary. Pass
/// the DWARF line-table sequences ([`crate::SymbolProvider::line_sequences`]):
/// they reach assembly that has line info but neither a sized symbol nor
/// mapping symbols. On ARM a range is only used when it starts in a `$t`
/// region, so A32 code is never decoded as Thumb.
pub fn decode_with_code_ranges(elf: &[u8], code_ranges: &[(u64, u64)]) -> BranchSites {
    let Ok(file) = object::File::parse(elf) else {
        return BranchSites::default();
    };
    let isa = match file.architecture() {
        object::Architecture::Arm => BranchIsa::Thumb,
        object::Architecture::Riscv32 | object::Architecture::Riscv64 => BranchIsa::RiscV,
        _ => return BranchSites::default(),
    };

    // Mapping symbols per section: (address, kind), ascending. ARM uses
    // $a/$t/$d, RISC-V uses $x/$d, each optionally suffixed (`$d.42`).
    let mut maps: std::collections::HashMap<object::SectionIndex, Vec<(u64, Marker)>> =
        std::collections::HashMap::new();
    for sym in file.symbols() {
        let (Ok(name), Some(sec)) = (sym.name(), sym.section_index()) else {
            continue;
        };
        let Some(marker) = marker_of(name) else {
            continue;
        };
        maps.entry(sec).or_default().push((sym.address(), marker));
    }
    for v in maps.values_mut() {
        v.sort_unstable_by_key(|(a, _)| *a);
    }

    // Ranges to walk, each starting on a known instruction boundary:
    // 1. every sized function symbol (on ARM, Thumb ones only);
    // 2. every code region a mapping symbol opens (`$t` on ARM, `$x` on
    //    RISC-V), up to the next mapping symbol. This reaches hand-written
    //    assembly, which often has line info but no sized symbol.
    let mut ranges: Vec<(object::SectionIndex, u64, u64)> = Vec::new();
    for sym in file.symbols() {
        if sym.kind() != SymbolKind::Text || sym.size() == 0 {
            continue;
        }
        let Some(sec) = sym.section_index() else {
            continue;
        };
        // A32 code would need a different decoder.
        if isa == BranchIsa::Thumb && sym.address() & 1 == 0 {
            continue;
        }
        let start = sym.address() & !1;
        ranges.push((sec, start, start.saturating_add(sym.size())));
    }
    for sym in file.symbols() {
        let (Ok(name), Some(sec)) = (sym.name(), sym.section_index()) else {
            continue;
        };
        let code_marker = match isa {
            BranchIsa::Thumb => Marker::Thumb,
            BranchIsa::RiscV => Marker::RiscV,
        };
        if marker_of(name) != Some(code_marker) {
            continue;
        }
        let Ok(section) = file.section_by_index(sec) else {
            continue;
        };
        let sec_end = section.address() + section.size();
        let start = sym.address();
        let end = maps
            .get(&sec)
            .and_then(|m| m.iter().find(|(a, _)| *a > start).map(|(a, _)| *a))
            .unwrap_or(sec_end)
            .min(sec_end);
        ranges.push((sec, start, end));
    }

    for &(start, end) in code_ranges {
        let Some(section) = file.sections().find(|sec| {
            sec.kind() == SectionKind::Text
                && start >= sec.address()
                && start < sec.address() + sec.size()
        }) else {
            continue;
        };
        let sec = section.index();
        if isa == BranchIsa::Thumb {
            let map = maps.get(&sec).map(Vec::as_slice).unwrap_or(&[]);
            if marker_at(map, start) != Some(Marker::Thumb) {
                continue;
            }
        }
        ranges.push((sec, start, end.min(section.address() + section.size())));
    }

    let mut sites = Vec::new();
    for (sec_idx, start, end) in ranges {
        let Ok(section) = file.section_by_index(sec_idx) else {
            continue;
        };
        if section.kind() != SectionKind::Text {
            continue;
        }
        let Ok(bytes) = section.data() else {
            continue;
        };
        let map = maps.get(&sec_idx).map(Vec::as_slice).unwrap_or(&[]);
        let read16 = |addr: u64| -> Option<u16> {
            let off = usize::try_from(addr.checked_sub(section.address())?).ok()?;
            let b = bytes.get(off..off + 2)?;
            Some(u16::from_le_bytes([b[0], b[1]]))
        };

        let mut pc = start;
        while pc.saturating_add(2) <= end {
            if let Some(next_code) = data_until(map, pc) {
                // Inside a literal pool: resume at the next code marker, or
                // stop if the pool runs to the end of the function.
                if next_code <= pc || next_code >= end {
                    break;
                }
                pc = next_code;
                continue;
            }
            let Some(lo) = read16(pc) else { break };
            let len: u64 = match isa {
                BranchIsa::Thumb => {
                    if (lo >> 11) >= 0b11101 {
                        4
                    } else {
                        2
                    }
                }
                BranchIsa::RiscV => {
                    if lo & 0b11 == 0b11 {
                        4
                    } else {
                        2
                    }
                }
            };
            if pc + len > end {
                break;
            }
            let hi = if len == 4 { read16(pc + 2) } else { Some(0) };
            let Some(hi) = hi else { break };
            let target = match isa {
                BranchIsa::Thumb => thumb_cond_branch(pc, lo, hi, len),
                BranchIsa::RiscV => riscv_cond_branch(pc, lo, hi, len),
            };
            if let Some(target) = target {
                // A branch to the very next instruction has one outcome;
                // there is nothing to cover.
                if target != pc + len {
                    sites.push(StaticBranch {
                        addr: pc,
                        len: len as u8,
                        target,
                    });
                }
            }
            pc += len;
        }
    }
    sites.sort_unstable_by_key(|s| s.addr);
    sites.dedup_by_key(|s| s.addr);
    BranchSites {
        isa: Some(isa),
        sites,
    }
}

/// What a mapping symbol says the bytes after it are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Data,
    Thumb,
    Arm,
    RiscV,
}

/// What a mapping-symbol name marks. ARM: `$a`, `$t`, `$d`, optionally
/// `.N`-suffixed. RISC-V: `$d`, and `$x` optionally followed by `.N` or by
/// the ISA string newer toolchains append (`$xrv32i2p1_m2p0`).
fn marker_of(name: &str) -> Option<Marker> {
    let plain = |tag: &str| name == tag || name.starts_with(&format!("{tag}."));
    if plain("$d") {
        Some(Marker::Data)
    } else if plain("$t") {
        Some(Marker::Thumb)
    } else if plain("$a") {
        Some(Marker::Arm)
    } else if name.starts_with("$x") {
        Some(Marker::RiscV)
    } else {
        None
    }
}

/// The mapping symbol in force at `addr` (the last one at or before it).
fn marker_at(map: &[(u64, Marker)], addr: u64) -> Option<Marker> {
    let idx = map.partition_point(|(a, _)| *a <= addr);
    Some(map.get(idx.checked_sub(1)?)?.1)
}

/// If `addr` is inside a `$d` region, the address where code resumes (the
/// next non-data marker, or `u64::MAX` when none follows). `None` when `addr`
/// is code.
fn data_until(map: &[(u64, Marker)], addr: u64) -> Option<u64> {
    if marker_at(map, addr)? != Marker::Data {
        return None;
    }
    let idx = map.partition_point(|(a, _)| *a <= addr);
    Some(
        map[idx..]
            .iter()
            .find(|(_, m)| *m != Marker::Data)
            .map(|(a, _)| *a)
            .unwrap_or(u64::MAX),
    )
}

fn sign_extend(value: u32, bits: u32) -> i64 {
    let shift = 32 - bits;
    i64::from(((value << shift) as i32) >> shift)
}

fn offset(pc: u64, base: u64, imm: i64) -> u64 {
    (pc + base).wrapping_add_signed(imm)
}

/// Target of a Thumb conditional branch at `pc`, or `None` if the
/// instruction is not one. `lo` is the first half-word, `hi` the second.
fn thumb_cond_branch(pc: u64, lo: u16, hi: u16, len: u64) -> Option<u64> {
    let (lo, hi) = (u32::from(lo), u32::from(hi));
    if len == 2 {
        // B<cond> T1: 1101 cccc iiiiiiii. cond 1110 is UDF, 1111 is SVC.
        if lo & 0xF000 == 0xD000 && (lo >> 8) & 0xF < 0xE {
            return Some(offset(pc, 4, sign_extend((lo & 0xFF) << 1, 9)));
        }
        // CBZ / CBNZ: 1011 o0i1 iiiii nnn. Forward only.
        if lo & 0xF500 == 0xB100 {
            let i = (lo >> 9) & 1;
            let imm5 = (lo >> 3) & 0x1F;
            return Some(pc + 4 + u64::from((i << 6) | (imm5 << 1)));
        }
        return None;
    }
    // B<cond> T3: 11110 S cccc iiiiii | 10 J1 0 J2 iiiiiiiiiii.
    // cond 111x is not a branch here (it encodes other instructions).
    if lo & 0xF800 == 0xF000 && hi & 0xD000 == 0x8000 && (lo >> 6) & 0xF < 0xE {
        let s = (lo >> 10) & 1;
        let imm6 = lo & 0x3F;
        let j1 = (hi >> 13) & 1;
        let j2 = (hi >> 11) & 1;
        let imm11 = hi & 0x7FF;
        let imm = (s << 20) | (j2 << 19) | (j1 << 18) | (imm6 << 12) | (imm11 << 1);
        return Some(offset(pc, 4, sign_extend(imm, 21)));
    }
    None
}

/// Target of a RISC-V conditional branch at `pc`, or `None`.
fn riscv_cond_branch(pc: u64, lo: u16, hi: u16, len: u64) -> Option<u64> {
    if len == 2 {
        let c = u32::from(lo);
        // C.BEQZ / C.BNEZ: quadrant 01, funct3 110 / 111.
        if c & 0b11 == 0b01 && (c >> 13) >= 0b110 {
            let imm = (((c >> 12) & 1) << 8)
                | (((c >> 10) & 0b11) << 3)
                | (((c >> 5) & 0b11) << 6)
                | (((c >> 3) & 0b11) << 1)
                | (((c >> 2) & 1) << 5);
            return Some(offset(pc, 0, sign_extend(imm, 9)));
        }
        return None;
    }
    let insn = u32::from(lo) | (u32::from(hi) << 16);
    // B-type: opcode 1100011; funct3 010 / 011 are reserved.
    let funct3 = (insn >> 12) & 0b111;
    if insn & 0x7F == 0x63 && funct3 != 0b010 && funct3 != 0b011 {
        let imm = (((insn >> 31) & 1) << 12)
            | (((insn >> 7) & 1) << 11)
            | (((insn >> 25) & 0x3F) << 5)
            | (((insn >> 8) & 0xF) << 1);
        return Some(offset(pc, 0, sign_extend(imm, 13)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every encoding below was assembled with llvm-mc 18 (thumbv7m / riscv32+c)
    // and the target read back from llvm-objdump.

    #[test]
    fn thumb_t1_conditional_branch() {
        // 0x1000: beq.n 0x1008  -> d002
        assert_eq!(thumb_cond_branch(0x1000, 0xD002, 0, 2), Some(0x1008));
        // bne.n backwards by 4 from 0x1010 -> 0x1010+4-4 = 0x1010: d1fe is b . (-4)
        assert_eq!(thumb_cond_branch(0x1010, 0xD1FE, 0, 2), Some(0x1010));
        // udf (cond 1110) and svc (cond 1111) are not branches.
        assert_eq!(thumb_cond_branch(0x1000, 0xDE00, 0, 2), None);
        assert_eq!(thumb_cond_branch(0x1000, 0xDF00, 0, 2), None);
        // Unconditional b.n (T2, 11100...) is not conditional.
        assert_eq!(thumb_cond_branch(0x1000, 0xE002, 0, 2), None);
    }

    #[test]
    fn thumb_cbz_cbnz() {
        // cbz r0, +8 from 0x2000 -> 0x200c: imm = 8 => i=0, imm5=4 -> b120
        assert_eq!(thumb_cond_branch(0x2000, 0xB120, 0, 2), Some(0x200C));
        // cbnz r1, +64 at 0x40 -> 0x84: bb01 (i=1, imm5=0)
        assert_eq!(thumb_cond_branch(0x40, 0xBB01, 0, 2), Some(0x84));
        // push {..} (1011 010x) is not cbz.
        assert_eq!(thumb_cond_branch(0x2000, 0xB580, 0, 2), None);
    }

    #[test]
    fn thumb_t3_conditional_branch() {
        // beq.w +0x200 from 0x3000: target 0x3204.
        // imm = 0x200 -> imm11 = 0x100, imm6 = 0, S=J1=J2=0 -> f000 8100
        assert_eq!(thumb_cond_branch(0x3000, 0xF000, 0x8100, 4), Some(0x3204));
        // bne.w -0x100 from 0x3000: target 0x2F04. S=1, J1=J2=1, imm6=0x3F,
        // imm11=0x780 -> f47f afXX: lo = 11110 1 0001 111111 = f47f,
        // hi = 10 1 0 1 11110000000 = af80
        assert_eq!(thumb_cond_branch(0x3000, 0xF47F, 0xAF80, 4), Some(0x2F04));
        // bl (hi bit 14 set) is not a conditional branch.
        assert_eq!(thumb_cond_branch(0x3000, 0xF000, 0xF800, 4), None);
        // cond 1110 in T3 slot encodes something else.
        assert_eq!(thumb_cond_branch(0x3000, 0xF380, 0x8000, 4), None);
    }

    #[test]
    fn riscv_b_type() {
        // beq a0, a1, +8 at 0x100: 00b50463
        assert_eq!(riscv_cond_branch(0x100, 0x0463, 0x00B5, 4), Some(0x108));
        // bne a0, zero, -4 at 0x100: fe051ee3
        assert_eq!(riscv_cond_branch(0x100, 0x1EE3, 0xFE05, 4), Some(0xFC));
        // jal (1101111) is not conditional.
        assert_eq!(riscv_cond_branch(0x100, 0x00EF, 0x0080, 4), None);
        // addi is not a branch.
        assert_eq!(riscv_cond_branch(0x100, 0x0513, 0x0015, 4), None);
    }

    #[test]
    fn riscv_compressed_branches() {
        // c.beqz a0, +8 at 0x200: c501
        assert_eq!(riscv_cond_branch(0x200, 0xC501, 0, 2), Some(0x208));
        // c.bnez a0, -2 at 0x200: fd7d
        assert_eq!(riscv_cond_branch(0x200, 0xFD7D, 0, 2), Some(0x1FE));
        // c.j (funct3 101) is unconditional.
        assert_eq!(riscv_cond_branch(0x200, 0xA001, 0, 2), None);
    }

    #[test]
    fn data_regions_follow_mapping_symbols() {
        use Marker::*;
        let map = [(0x100, Thumb), (0x120, Data), (0x128, Thumb), (0x140, Data)];
        assert_eq!(data_until(&map, 0x100), None);
        assert_eq!(data_until(&map, 0x124), Some(0x128));
        assert_eq!(data_until(&map, 0x12A), None);
        assert_eq!(data_until(&map, 0x150), Some(u64::MAX));
        assert_eq!(data_until(&map, 0x50), None, "before any marker is code");
        assert_eq!(marker_at(&map, 0x12A), Some(Thumb));
        assert_eq!(marker_at(&map, 0x50), None);
    }

    #[test]
    fn mapping_symbol_names() {
        assert_eq!(marker_of("$d"), Some(Marker::Data));
        assert_eq!(marker_of("$d.17"), Some(Marker::Data));
        assert_eq!(marker_of("$t.3"), Some(Marker::Thumb));
        assert_eq!(marker_of("$a"), Some(Marker::Arm));
        assert_eq!(marker_of("$x"), Some(Marker::RiscV));
        assert_eq!(marker_of("$xrv32i2p1_m2p0_c2p0"), Some(Marker::RiscV));
        assert_eq!(marker_of("$data_start"), None);
        assert_eq!(marker_of("$dummy"), None);
        assert_eq!(marker_of("main"), None);
    }
}
