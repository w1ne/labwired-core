// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Env-gated JIT coverage diagnostics: *why* did dispatch interpret this
//! instruction instead of running compiled code?
//!
//! Coverage (the fraction of retired instructions that land in compiled
//! blocks) is the Amdahl ceiling for the whole JIT. When it is low, the only
//! useful question is which cause is responsible for the *retired
//! instructions* lost — not which cause is most common at translate time.
//! This module answers exactly that, ranked, on a real firmware run.
//!
//! It is **off** unless `LW_JIT_CUT_STATS=1` is set, and when off costs one
//! relaxed atomic load per interpreted instruction. It is a diagnostic, not a
//! product surface: nothing in the emitted code, the block cache, or the
//! dispatch decisions reads it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Why one dispatch landed on the interpreter rather than a compiled block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InterpReason {
    /// The PC has not yet reached the block cache's hot threshold.
    NotHotYet,
    /// The PC was promoted but the frontend produced a body-less stub, i.e.
    /// the *entry* instruction itself is unmodeled.
    EntryUnmodeled,
    /// The frontend refused the PC outright (out of fetchable code memory).
    FrontendRefused,
    /// A real block was emitted but was shorter than the profitability floor.
    BelowMinBlock,
    /// wasm validation / instantiation failed.
    CompileFailed,
    /// A block is ready but retiring it would run past the batch budget.
    BatchBudget,
    /// A block is ready but would step across an interrupt deadline.
    IrqCross,
    /// A block is ready but retires zero instructions.
    ZeroLength,
    /// A compiled block took a mid-block memory fault at its entry.
    EntryMemFault,
}

/// One tallied cut cause plus the instruction that terminated the walk.
#[derive(Debug, Clone, Default)]
pub struct Tally {
    /// Interpreted instructions attributed to this cause.
    pub interpreted: u64,
    /// Length of the block the frontend produced at this PC (0 = stub).
    pub block_len: u32,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static INIT: OnceLock<()> = OnceLock::new();

#[allow(clippy::type_complexity)]
fn table() -> &'static Mutex<HashMap<(InterpReason, &'static str), Tally>> {
    static T: OnceLock<Mutex<HashMap<(InterpReason, &'static str), Tally>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether diagnostics are on (`LW_JIT_CUT_STATS=1`).
#[inline]
pub fn enabled() -> bool {
    INIT.get_or_init(|| {
        let on = std::env::var("LW_JIT_CUT_STATS").as_deref() == Ok("1");
        ENABLED.store(on, Ordering::Relaxed);
    });
    ENABLED.load(Ordering::Relaxed)
}

/// Record one interpreted instruction against `reason`, blamed on the
/// instruction mnemonic `blame` that terminated the block walk at this PC
/// (`"-"` when the cause is not a walk termination).
pub fn note(reason: InterpReason, blame: &'static str, block_len: u32) {
    if !enabled() {
        return;
    }
    let mut t = table().lock().expect("cutstats lock");
    let e = t.entry((reason, blame)).or_default();
    e.interpreted += 1;
    e.block_len = block_len;
}

#[allow(clippy::type_complexity)]
fn word_table() -> &'static Mutex<HashMap<(Pc, u32), u64>> {
    static T: OnceLock<Mutex<HashMap<(Pc, u32), u64>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

use super::Pc;

/// Record the raw word (and PC) of an instruction that terminated a walk, so
/// the report can name *what* the undecodable / unmodeled bytes actually are.
pub fn note_word(pc: Pc, word: u32) {
    if !enabled() {
        return;
    }
    *word_table()
        .lock()
        .expect("word lock")
        .entry((pc, word))
        .or_insert(0) += 1;
}

/// Drain the (pc,word) histogram, most-frequent first.
pub fn word_report() -> Vec<((Pc, u32), u64)> {
    let t = word_table().lock().expect("word lock");
    let mut v: Vec<_> = t.iter().map(|(k, c)| (*k, *c)).collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1));
    v
}

/// Drain the table into a report ranked by interpreted-instruction cost.
pub fn report() -> Vec<((InterpReason, &'static str), Tally)> {
    let t = table().lock().expect("cutstats lock");
    let mut v: Vec<_> = t.iter().map(|(k, val)| (*k, val.clone())).collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1.interpreted));
    v
}

/// Clear the table (between measurement arms).
pub fn reset() {
    table().lock().expect("cutstats lock").clear();
}

/// Stable mnemonic for a decoded instruction — the blame label in the report.
pub fn mnemonic(inst: &crate::decoder::riscv::Instruction) -> &'static str {
    use crate::decoder::riscv::Instruction::*;
    match inst {
        Fence => "fence",
        Ecall => "ecall",
        Ebreak => "ebreak",
        Mret => "mret",
        Wfi => "wfi",
        Csrrw { .. } => "csrrw",
        Csrrs { .. } => "csrrs",
        Csrrc { .. } => "csrrc",
        Csrrwi { .. } => "csrrwi",
        Csrrsi { .. } => "csrrsi",
        Csrrci { .. } => "csrrci",
        LrW { .. } => "lr.w",
        ScW { .. } => "sc.w",
        AmoSwapW { .. } => "amoswap.w",
        AmoAddW { .. } => "amoadd.w",
        AmoXorW { .. } => "amoxor.w",
        AmoOrW { .. } => "amoor.w",
        AmoAndW { .. } => "amoand.w",
        AmoMinW { .. } => "amomin.w",
        AmoMaxW { .. } => "amomax.w",
        AmoMinuW { .. } => "amominu.w",
        AmoMaxuW { .. } => "amomaxu.w",
        Unknown(_) => "unknown",
        Beq { .. } | Bne { .. } | Blt { .. } | Bge { .. } | Bltu { .. } | Bgeu { .. } => "branch",
        CBeqz { .. } | CBnez { .. } => "c.branch",
        Jal { .. } | CJ { .. } => "jal",
        Jalr { .. } | CJr { .. } | CJalr { .. } => "jalr",
        _ => "sequential",
    }
}
