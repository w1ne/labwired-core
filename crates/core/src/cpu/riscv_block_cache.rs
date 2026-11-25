// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Decoded RV32IMC basic-block cache design (direct-mapped block table,
// contiguous decoded-step arena, store-tracking flags, and pure self-loop
// fast path) adapted from `mikroverk/esp32sim` (`riscv-rv32/src/block.rs`,
// EX265):
// Copyright (c) 2026 Joakim Eriksson (@joakimeriksson),
// Alice (@aliceisjustplaying) and Mattia Fiumara (@mfiumara) — MIT License.

use crate::decoder::riscv::{decode_qingke_xw, decode_rv32, Instruction};

use super::riscv::{rv32e_uses_high_register, RiscVCoreProfile};

const TABLE_BITS: usize = 11;
const TABLE_ENTRIES: usize = 1 << TABLE_BITS;
const TABLE_MASK: usize = TABLE_ENTRIES - 1;

/// Maximum instructions compiled into one decoded basic block.
pub const MAX_BLOCK_LEN: usize = 32;

/// Arena capacity before bumping the generation counter and reclaiming space.
const MAX_ARENA_LEN: usize = 32 * 1024;

/// Block contains at least one load or store instruction.
pub const FLAG_HAS_MEM: u8 = 1 << 0;
/// Block contains at least one store instruction.
pub const FLAG_HAS_STORE: u8 = 1 << 1;
/// Block ends in a conditional/unconditional branch or jump targeting `tag_pc`
/// and performs no memory stores (safe for tight self-loop execution).
pub const FLAG_SELF_LOOP: u8 = 1 << 2;
/// Entry word must be re-verified against guest memory (used only for
/// synthetic base-zero flash where debugger/host tests patch words directly).
pub const FLAG_VERIFY_ENTRY: u8 = 1 << 3;
/// Block was fetched from writable memory (RAM / extra_mem) and must be
/// invalidated if a guest store overlaps its byte span.
pub const FLAG_WRITABLE_CODE: u8 = 1 << 4;

/// One pre-decoded instruction inside a cached basic block.
#[derive(Debug, Clone, Copy)]
pub struct DecodedStep {
    pub inst: Instruction,
    pub pc: u32,
    pub next_pc: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct BlockSlot {
    tag_pc: u32,
    gen: u32,
    start: u32,
    len: u8,
    flags: u8,
    entry_word: u32,
}

/// Lookup handle into [`Rv32BlockCache::arena`].
#[derive(Debug, Clone, Copy)]
pub struct CachedBlockMeta {
    pub tag_pc: u32,
    pub start: usize,
    pub len: usize,
    pub flags: u8,
    pub entry_word: u32,
}

/// Direct-mapped decoded basic-block cache for RV32IMAC/RV32E.
///
/// Works on every host architecture (x86-64, AArch64, and browser WASM32)
/// without JIT compilation dependencies or warmup cost.
#[derive(Debug, Clone)]
pub struct Rv32BlockCache {
    gen: u32,
    slots: Box<[BlockSlot; TABLE_ENTRIES]>,
    arena: Vec<DecodedStep>,
    has_writable_blocks: bool,
    writable_min_pc: u32,
    writable_max_end: u32,
}

impl Default for Rv32BlockCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Rv32BlockCache {
    pub fn new() -> Self {
        Self {
            gen: 1,
            slots: Box::new([BlockSlot::default(); TABLE_ENTRIES]),
            arena: Vec::with_capacity(4096),
            has_writable_blocks: false,
            writable_min_pc: u32::MAX,
            writable_max_end: 0,
        }
    }

    /// Drop every cached block in O(1) by advancing the generation counter.
    #[inline]
    pub fn invalidate(&mut self) {
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            self.gen = 1;
            self.slots.fill(BlockSlot::default());
        }
        self.arena.clear();
        self.has_writable_blocks = false;
        self.writable_min_pc = u32::MAX;
        self.writable_max_end = 0;
    }

    /// Invalidate cached blocks if a guest store `[addr, addr + size)` overlaps
    /// any block that was compiled from writable memory (IRAM / SRAM).
    #[inline]
    pub fn invalidate_if_store_overlaps(&mut self, addr: u32, size: u32) {
        if !self.has_writable_blocks || size == 0 {
            return;
        }
        let store_hi = addr.wrapping_add(size);
        if addr < self.writable_max_end && store_hi > self.writable_min_pc {
            self.invalidate();
        }
    }

    /// O(1) direct-mapped lookup for a block starting at `pc`.
    #[inline]
    pub fn lookup(&self, pc: u32) -> Option<CachedBlockMeta> {
        let idx = ((pc >> 1) as usize) & TABLE_MASK;
        let slot = self.slots[idx];
        if slot.gen == self.gen && slot.tag_pc == pc && slot.len > 0 {
            Some(CachedBlockMeta {
                tag_pc: slot.tag_pc,
                start: slot.start as usize,
                len: slot.len as usize,
                flags: slot.flags,
                entry_word: slot.entry_word,
            })
        } else {
            None
        }
    }

    /// Borrow the pre-decoded instruction slice for a validated [`CachedBlockMeta`].
    #[inline]
    pub fn steps(&self, meta: CachedBlockMeta) -> &[DecodedStep] {
        &self.arena[meta.start..meta.start + meta.len]
    }

    /// Decode a straight-line basic block starting at `start_pc` from
    /// `code_bytes` (a byte-exact prefix of guest instruction memory at
    /// `start_pc`) and install it into the cache.
    ///
    /// Returns `None` if the entry instruction itself must be handled by the
    /// full `RiscV::step` path (CSR, trap, atomic, `wfi`, `mret`, illegal, etc.).
    pub fn compile_block(
        &mut self,
        start_pc: u32,
        code_bytes: &[u8],
        core_profile: RiscVCoreProfile,
        writable_code: bool,
        verify_entry: bool,
    ) -> Option<CachedBlockMeta> {
        if code_bytes.len() < 2 {
            return None;
        }
        if self.arena.len() + MAX_BLOCK_LEN > MAX_ARENA_LEN {
            self.invalidate();
        }

        let start_idx = self.arena.len();
        let mut off: usize = 0;
        let mut pc = start_pc;
        let mut flags: u8 = 0;
        if writable_code {
            flags |= FLAG_WRITABLE_CODE;
        }
        if verify_entry {
            flags |= FLAG_VERIFY_ENTRY;
        }

        let entry_word = if code_bytes.len() >= 4 {
            u32::from_le_bytes([code_bytes[0], code_bytes[1], code_bytes[2], code_bytes[3]])
        } else {
            u32::from(u16::from_le_bytes([code_bytes[0], code_bytes[1]]))
        };

        let mut count: usize = 0;
        while count < MAX_BLOCK_LEN && off + 2 <= code_bytes.len() {
            let low16 = u16::from_le_bytes([code_bytes[off], code_bytes[off + 1]]);
            let is_32 = (low16 & 0x3) == 0x3;
            let inst_len: usize = if is_32 { 4 } else { 2 };
            if off + inst_len > code_bytes.len() {
                break;
            }
            let opcode = if is_32 {
                u32::from_le_bytes([
                    code_bytes[off],
                    code_bytes[off + 1],
                    code_bytes[off + 2],
                    code_bytes[off + 3],
                ])
            } else {
                u32::from(low16)
            };

            let mut inst = decode_rv32(opcode);
            if !is_32
                && core_profile == RiscVCoreProfile::Rv32e
                && matches!(inst, Instruction::Unknown(_))
            {
                if let Some(xw) = decode_qingke_xw(low16) {
                    inst = xw;
                }
            }

            if core_profile == RiscVCoreProfile::Rv32e && rv32e_uses_high_register(&inst) {
                break;
            }

            match classify(&inst) {
                StepKind::Alu => {}
                StepKind::Load => {
                    flags |= FLAG_HAS_MEM;
                }
                StepKind::Store => {
                    flags |= FLAG_HAS_MEM | FLAG_HAS_STORE;
                }
                StepKind::BranchOrJump { self_loop_offset } => {
                    let next_pc = pc.wrapping_add(inst_len as u32);
                    if let Some(rel) = self_loop_offset {
                        if pc.wrapping_add(rel) == start_pc && (flags & FLAG_HAS_STORE) == 0 {
                            flags |= FLAG_SELF_LOOP;
                        }
                    }
                    self.arena.push(DecodedStep { inst, pc, next_pc });
                    count += 1;
                    off += inst_len;
                    break;
                }
                StepKind::Unsupported => {
                    break;
                }
            }

            let next_pc = pc.wrapping_add(inst_len as u32);
            self.arena.push(DecodedStep { inst, pc, next_pc });
            count += 1;
            off += inst_len;
            pc = next_pc;
        }

        if count == 0 {
            return None;
        }

        let span_end = start_pc.wrapping_add(off as u32);
        if writable_code {
            self.has_writable_blocks = true;
            if start_pc < self.writable_min_pc {
                self.writable_min_pc = start_pc;
            }
            if span_end > self.writable_max_end {
                self.writable_max_end = span_end;
            }
        }

        let idx = ((start_pc >> 1) as usize) & TABLE_MASK;
        self.slots[idx] = BlockSlot {
            tag_pc: start_pc,
            gen: self.gen,
            start: start_idx as u32,
            len: count as u8,
            flags,
            entry_word,
        };

        Some(CachedBlockMeta {
            tag_pc: start_pc,
            start: start_idx,
            len: count,
            flags,
            entry_word,
        })
    }
}

enum StepKind {
    Alu,
    Load,
    Store,
    BranchOrJump { self_loop_offset: Option<u32> },
    Unsupported,
}

#[inline]
fn classify(inst: &Instruction) -> StepKind {
    match *inst {
        Instruction::Lui { .. }
        | Instruction::Auipc { .. }
        | Instruction::Addi { .. }
        | Instruction::Slti { .. }
        | Instruction::Sltiu { .. }
        | Instruction::Xori { .. }
        | Instruction::Ori { .. }
        | Instruction::Andi { .. }
        | Instruction::Slli { .. }
        | Instruction::Srli { .. }
        | Instruction::Srai { .. }
        | Instruction::Add { .. }
        | Instruction::Sub { .. }
        | Instruction::Sll { .. }
        | Instruction::Slt { .. }
        | Instruction::Sltu { .. }
        | Instruction::Xor { .. }
        | Instruction::Srl { .. }
        | Instruction::Sra { .. }
        | Instruction::Or { .. }
        | Instruction::And { .. }
        | Instruction::Fence
        | Instruction::Mul { .. }
        | Instruction::Mulh { .. }
        | Instruction::Mulhsu { .. }
        | Instruction::Mulhu { .. }
        | Instruction::Div { .. }
        | Instruction::Divu { .. }
        | Instruction::Rem { .. }
        | Instruction::Remu { .. }
        | Instruction::CAddi { .. }
        | Instruction::CLi { .. }
        | Instruction::CMv { .. }
        | Instruction::CAddi16sp { .. }
        | Instruction::CAddi4spn { .. }
        | Instruction::CSli { .. } => StepKind::Alu,

        Instruction::Lb { .. }
        | Instruction::Lh { .. }
        | Instruction::Lw { .. }
        | Instruction::Lbu { .. }
        | Instruction::Lhu { .. }
        | Instruction::CLw { .. }
        | Instruction::CLwsp { .. } => StepKind::Load,

        Instruction::Sb { .. }
        | Instruction::Sh { .. }
        | Instruction::Sw { .. }
        | Instruction::CSw { .. }
        | Instruction::CSwsp { .. } => StepKind::Store,

        Instruction::Beq { imm, .. }
        | Instruction::Bne { imm, .. }
        | Instruction::Blt { imm, .. }
        | Instruction::Bge { imm, .. }
        | Instruction::Bltu { imm, .. }
        | Instruction::Bgeu { imm, .. }
        | Instruction::CBeqz { imm, .. }
        | Instruction::CBnez { imm, .. }
        | Instruction::Jal { imm, .. }
        | Instruction::CJ { imm } => StepKind::BranchOrJump {
            self_loop_offset: Some(imm as u32),
        },
        Instruction::Jalr { .. } | Instruction::CJr { .. } | Instruction::CJalr { .. } => {
            StepKind::BranchOrJump {
                self_loop_offset: None,
            }
        }

        Instruction::Ecall
        | Instruction::Ebreak
        | Instruction::Mret
        | Instruction::Wfi
        | Instruction::Csrrw { .. }
        | Instruction::Csrrs { .. }
        | Instruction::Csrrc { .. }
        | Instruction::Csrrwi { .. }
        | Instruction::Csrrsi { .. }
        | Instruction::Csrrci { .. }
        | Instruction::LrW { .. }
        | Instruction::ScW { .. }
        | Instruction::AmoSwapW { .. }
        | Instruction::AmoAddW { .. }
        | Instruction::AmoXorW { .. }
        | Instruction::AmoOrW { .. }
        | Instruction::AmoAndW { .. }
        | Instruction::AmoMinW { .. }
        | Instruction::AmoMaxW { .. }
        | Instruction::AmoMinuW { .. }
        | Instruction::AmoMaxuW { .. }
        | Instruction::Unknown(_) => StepKind::Unsupported,
    }
}
