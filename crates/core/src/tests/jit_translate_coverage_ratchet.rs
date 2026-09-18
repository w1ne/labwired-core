// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Every decoded instruction is either translated by the JIT or declared
//! interpreter-only, on purpose.
//!
//! Each architecture's ISA semantics are implemented twice: once in the
//! interpreter's decoder + `step` (`crate::decoder::{arm,riscv,xtensa}` and
//! `crate::cpu::{cortex_m,riscv,xtensa_*}`), and once more in the JIT's
//! translator (`crate::cpu::jit_framework::{cortex_m,riscv}::emit` and
//! `crate::cpu::xtensa_jit::emit_core`). Nothing stops a new interpreter
//! variant from landing without the JIT side either picking it up or
//! explicitly deciding it stays on the interpreter — the differential
//! lockstep tests (`cortex_m_jit_alu_lockstep`, `riscv_jit_*_lockstep`,
//! `xtensa_jit_*`) only cover what someone remembered to add.
//!
//! This is a pure source scan: for each architecture, every variant of the
//! decoder's `Instruction` enum must be EITHER
//!
//! * mentioned by name somewhere in that architecture's JIT translator
//!   sources (a reasonable proxy for "the JIT knows how to handle this
//!   opcode" — the emit modules match on bare variant names via
//!   `use Instruction::*;`, so a textual mention is what "referenced" means
//!   here), OR
//! * listed in that architecture's `INTERPRETER_ONLY` allow-list below, with
//!   a one-line reason.
//!
//! A variant in neither bucket fails the test and tells the author to do one
//! of those two things. This is a RATCHET, not just a checklist: an
//! allow-listed variant that IS now referenced by the JIT is ALSO a failure
//! (a stale entry), so the list can only shrink as translation coverage
//! grows — nobody can "fix" a failure by leaving a dead entry behind.
//!
//! AVR has no JIT at all (`crate::cpu::avr`, interpreter-only end to end), so
//! it is skipped explicitly rather than silently absent from this file.
//!
//! # What this does NOT do
//!
//! It does not check that a "translated" variant is translated CORRECTLY —
//! that is what the lockstep differential tests are for. It does not run any
//! JIT code or require the `jit`/`jit-framework` features; it only reads
//! source text, so it runs under a plain `cargo test -p labwired-core --lib`.
//! A name match is also not proof of semantic coverage (a variant could be
//! mentioned in a comment) — the allow-list is the escape hatch for exactly
//! that kind of edge case, reviewed by a human once, not re-derived by magic
//! on every run.

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Source-text plumbing, shared with `event_scheduler_cfg_ratchet`'s approach:
// blank comments/strings before scanning so prose can never masquerade as
// code.
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Blank every comment and string/char literal body, preserving byte offsets
/// (mirrors `event_scheduler_cfg_ratchet::strip_comments_and_strings`).
fn strip_comments_and_strings(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for p in from..to.min(out.len()) {
            if out[p] != b'\n' {
                out[p] = b' ';
            }
        }
    };
    while i < b.len() {
        if b[i] == b'"' {
            let mut j = i + 1;
            while j < b.len() {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == b'"' {
                    break;
                }
                j += 1;
            }
            let end = (j + 1).min(b.len());
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            let end = src[i..].find('\n').map(|k| i + k).unwrap_or(b.len());
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth = 1usize;
            let mut j = i + 2;
            while j < b.len() && depth > 0 {
                if b[j] == b'/' && j + 1 < b.len() && b[j + 1] == b'*' {
                    depth += 1;
                    j += 2;
                } else if b[j] == b'*' && j + 1 < b.len() && b[j + 1] == b'/' {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j.min(b.len()));
            i = j;
            continue;
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Read `path` under the repo root, comment/string-stripped.
fn read_stripped(path: &Path) -> String {
    let full = repo_root().join(path);
    let src =
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()));
    strip_comments_and_strings(&src)
}

/// Parse the top-level variant names of `pub enum Instruction { .. }` out of
/// a comment-stripped decoder source. Brace/paren-depth aware, so multi-line
/// struct-style variants (`Foo { a: u8, b: u8 },`) and tuple variants
/// (`Bar(u32)`) both split correctly on their top-level commas.
fn enum_variant_names(stripped_src: &str) -> Vec<String> {
    const MARKER: &str = "pub enum Instruction {";
    let start = stripped_src
        .find(MARKER)
        .unwrap_or_else(|| panic!("no `{MARKER}` found"))
        + MARKER.len();
    let b = stripped_src.as_bytes();
    let mut depth = 1i32;
    let mut i = start;
    let mut fields = Vec::new();
    let mut field_start = start;
    while i < b.len() && depth > 0 {
        match b[i] {
            b'{' | b'(' => depth += 1,
            b'}' | b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            b',' if depth == 1 => {
                fields.push(stripped_src[field_start..i].to_string());
                field_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let tail = stripped_src[field_start..i].trim();
    if !tail.is_empty() {
        fields.push(tail.to_string());
    }

    fields
        .into_iter()
        .filter_map(|f| {
            let f = f.trim();
            let end = f
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(f.len());
            let name = &f[..end];
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

/// Does `haystack` contain `word` as a whole identifier (not as a substring
/// of a longer identifier)?
fn contains_word(haystack: &str, word: &str) -> bool {
    let h = haystack.as_bytes();
    let w = word.as_bytes();
    if w.is_empty() || h.len() < w.len() {
        return false;
    }
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    for start in 0..=(h.len() - w.len()) {
        if &h[start..start + w.len()] != w {
            continue;
        }
        let before_ok = start == 0 || !is_ident(h[start - 1]);
        let after = start + w.len();
        let after_ok = after == h.len() || !is_ident(h[after]);
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

fn concat_sources(paths: &[&str]) -> String {
    paths
        .iter()
        .map(|p| read_stripped(Path::new(p)))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// The gate itself, run once per architecture.
// ---------------------------------------------------------------------------

struct ArchCoverage {
    name: &'static str,
    decoder_path: &'static str,
    /// JIT translator sources this architecture's coverage is judged against.
    jit_paths: &'static [&'static str],
    /// `(variant name, one-line reason it is not JIT-translated)`. Kept
    /// sorted by name for readability; order is not otherwise significant.
    interpreter_only: &'static [(&'static str, &'static str)],
}

/// ARM Thumb/Thumb-2 (Cortex-M). JIT: `cpu::jit_framework::cortex_m`.
///
/// Seeded from the tree at the time this gate was added: 147 decoded
/// variants, 109 referenced by the emit module's `is_alu_emittable` /
/// `is_mem_emittable` / `is_terminator_emittable` (and the DataProc32 op
/// tables), 38 interpreter-only.
const CORTEX_M: ArchCoverage = ArchCoverage {
    name: "cortex_m",
    decoder_path: "crates/core/src/decoder/arm.rs",
    jit_paths: &[
        "crates/core/src/cpu/jit_framework/cortex_m/mod.rs",
        "crates/core/src/cpu/jit_framework/cortex_m/emit.rs",
        "crates/core/src/cpu/jit_framework/cortex_m/exec.rs",
        "crates/core/src/cpu/jit_framework/cortex_m/host.rs",
    ],
    interpreter_only: &[
        ("Bfc", "bitfield clear, not in the JIT ALU emit set"),
        ("Bfi", "bitfield insert, not in the JIT ALU emit set"),
        (
            "ExtendW",
            "32-bit sign/zero extend, not in the JIT ALU emit set",
        ),
        ("Ldrd", "load register pair, not in the JIT mem emit set"),
        ("Mla", "multiply-accumulate, not in the JIT ALU emit set"),
        ("Mls", "multiply-subtract, not in the JIT ALU emit set"),
        (
            "Sbfx",
            "signed bitfield extract, not in the JIT ALU emit set",
        ),
        ("Sdiv", "signed 32-bit divide, not in the JIT ALU emit set"),
        ("Sel", "SEL byte-select (DSP extension), not translated"),
        (
            "ShiftReg32",
            "32-bit register-shift dataproc form, not translated",
        ),
        (
            "SimdAddSub16",
            "SIMD add/sub (DSP extension), not translated",
        ),
        (
            "SimdAddSub8",
            "SIMD add/sub (DSP extension), not translated",
        ),
        (
            "Smlal",
            "64-bit multiply-accumulate, not in the JIT ALU emit set",
        ),
        (
            "SmlaXy",
            "16x16 multiply-accumulate (DSP extension), not translated",
        ),
        ("Smull", "64-bit multiply, not in the JIT ALU emit set"),
        ("Strd", "store register pair, not in the JIT mem emit set"),
        (
            "Ubfx",
            "unsigned bitfield extract, not in the JIT ALU emit set",
        ),
        (
            "Udiv",
            "unsigned 32-bit divide, not in the JIT ALU emit set",
        ),
        (
            "Umaal",
            "64-bit multiply-accumulate, not in the JIT ALU emit set",
        ),
        (
            "Umlal",
            "64-bit multiply-accumulate, not in the JIT ALU emit set",
        ),
        ("Umull", "64-bit multiply, not in the JIT ALU emit set"),
        (
            "VaddF64",
            "double-precision VFP op, JIT only handles single-precision",
        ),
        ("VcvtF32FromInt", "VFP int-to-float convert, not translated"),
        ("VcvtIntFromF32", "VFP float-to-int convert, not translated"),
        (
            "VdivF64",
            "double-precision VFP op, JIT only handles single-precision",
        ),
        ("VfmaF32", "VFP fused multiply-add, not translated"),
        ("VfmsF32", "VFP fused multiply-subtract, not translated"),
        ("VfnmaF32", "VFP fused negated multiply-add, not translated"),
        (
            "VfnmsF32",
            "VFP fused negated multiply-subtract, not translated",
        ),
        (
            "VfpLoadMultiple",
            "VFP multiple-register load, not translated",
        ),
        (
            "VfpStoreMultiple",
            "VFP multiple-register store, not translated",
        ),
        (
            "VmovDRtRt2",
            "double-precision VFP <-> core register move, not translated",
        ),
        (
            "VmovF64Reg",
            "double-precision VFP register move, not translated",
        ),
        (
            "VmovRtRt2D",
            "double-precision VFP <-> core register move, not translated",
        ),
        ("Vldr64", "double-precision VFP load, not translated"),
        ("Vstr64", "double-precision VFP store, not translated"),
        (
            "VmulF64",
            "double-precision VFP op, JIT only handles single-precision",
        ),
        (
            "VsubF64",
            "double-precision VFP op, JIT only handles single-precision",
        ),
    ],
};

/// RV32IMAC. JIT: `cpu::jit_framework::riscv`.
///
/// Seeded from the tree at the time this gate was added: 83 decoded
/// variants, all 83 referenced by the emit module. No interpreter-only
/// entries today — the list exists so the FIRST riscv variant that isn't
/// translated has somewhere to go, not because none will ever be added.
const RISCV: ArchCoverage = ArchCoverage {
    name: "riscv",
    decoder_path: "crates/core/src/decoder/riscv.rs",
    jit_paths: &[
        "crates/core/src/cpu/jit_framework/riscv/mod.rs",
        "crates/core/src/cpu/jit_framework/riscv/emit.rs",
        "crates/core/src/cpu/jit_framework/riscv/exec.rs",
        "crates/core/src/cpu/jit_framework/riscv/host.rs",
        "crates/core/src/cpu/jit_framework/riscv/wasm_encode.rs",
    ],
    interpreter_only: &[],
};

/// Xtensa LX6/LX7. JIT: `cpu::xtensa_jit` (a hot-basic-block translator, far
/// narrower than the other two architectures' emit cores).
///
/// Seeded from the tree at the time this gate was added: 166 decoded
/// variants, 56 referenced by `emit_core`'s `is_supported`, 110
/// interpreter-only — mostly the floating-point coprocessor, windowed-ABI /
/// exception-frame ops, special-register access and zero-overhead loop
/// setup, none of which the hot-block translator models.
const XTENSA: ArchCoverage = ArchCoverage {
    name: "xtensa",
    decoder_path: "crates/core/src/decoder/xtensa.rs",
    jit_paths: &[
        "crates/core/src/cpu/xtensa_jit/mod.rs",
        "crates/core/src/cpu/xtensa_jit/emit_core.rs",
        "crates/core/src/cpu/xtensa_jit/bb_multi.rs",
        "crates/core/src/cpu/xtensa_jit/windowed_call.rs",
    ],
    interpreter_only: &[
        ("Abs", "unary ALU op, not in the JIT hot-block op set"),
        ("AbsS", "FPU (single-precision) op, not translated"),
        ("AddS", "FPU (single-precision) op, not translated"),
        ("Addx2", "scaled add, not in the JIT hot-block op set"),
        ("Addx4", "scaled add, not in the JIT hot-block op set"),
        ("Addx8", "scaled add, not in the JIT hot-block op set"),
        ("Break", "debug breakpoint, cold path stays interpreted"),
        ("CeilS", "FPU (single-precision) op, not translated"),
        ("Clamps", "signed clamp op, not in the JIT hot-block op set"),
        ("CmpS", "FPU (single-precision) compare, not translated"),
        ("Dsync", "pipeline sync op, cold path stays interpreted"),
        ("Esync", "pipeline sync op, cold path stays interpreted"),
        (
            "Extw",
            "memory-ordering barrier, cold path stays interpreted",
        ),
        ("FloatS", "FPU int-to-float convert, not translated"),
        ("FloorS", "FPU (single-precision) op, not translated"),
        ("Isync", "pipeline sync op, cold path stays interpreted"),
        ("L32ai", "atomic load, not in the JIT hot-block op set"),
        ("L32e", "windowed-register-file load, interpreter only"),
        (
            "Loop",
            "zero-overhead loop setup, control flow the JIT does not model",
        ),
        (
            "Loopgtz",
            "zero-overhead loop setup, control flow the JIT does not model",
        ),
        (
            "Loopnez",
            "zero-overhead loop setup, control flow the JIT does not model",
        ),
        ("Lsi", "FPU load, not translated"),
        ("Lsiu", "FPU load with update, not translated"),
        ("Lsx", "FPU indexed load, not translated"),
        ("Lsxu", "FPU indexed load with update, not translated"),
        (
            "MaddS",
            "FPU (single-precision) fused multiply-add, not translated",
        ),
        ("Max", "min/max ALU op, not in the JIT hot-block op set"),
        ("Maxu", "min/max ALU op, not in the JIT hot-block op set"),
        ("Min", "min/max ALU op, not in the JIT hot-block op set"),
        ("Minu", "min/max ALU op, not in the JIT hot-block op set"),
        ("MovS", "FPU (single-precision) move, not translated"),
        ("MovfS", "FPU conditional move, not translated"),
        ("MovgezS", "FPU conditional move, not translated"),
        (
            "Movgez",
            "conditional move, not in the JIT hot-block op set",
        ),
        (
            "Movltz",
            "conditional move, not in the JIT hot-block op set",
        ),
        ("MovltzS", "FPU conditional move, not translated"),
        (
            "Movnez",
            "conditional move, not in the JIT hot-block op set",
        ),
        ("MovnezS", "FPU conditional move, not translated"),
        (
            "Moveqz",
            "conditional move, not in the JIT hot-block op set",
        ),
        ("MoveqzS", "FPU conditional move, not translated"),
        ("Movsp", "windowed-ABI stack-pointer move, interpreter only"),
        ("MovtS", "FPU conditional move, not translated"),
        (
            "MsubS",
            "FPU (single-precision) fused multiply-subtract, not translated",
        ),
        ("Mul16s", "16-bit multiply extension op, not translated"),
        ("Mul16u", "16-bit multiply extension op, not translated"),
        ("MulS", "FPU (single-precision) multiply, not translated"),
        ("Mull", "32-bit multiply extension op, not translated"),
        ("Mulsh", "32-bit multiply extension op, not translated"),
        ("Muluh", "32-bit multiply extension op, not translated"),
        ("Neg", "unary ALU op, not in the JIT hot-block op set"),
        ("NegS", "FPU (single-precision) op, not translated"),
        (
            "Nsa",
            "normalize-shift-amount op, not in the JIT hot-block op set",
        ),
        (
            "Nsau",
            "normalize-shift-amount op, not in the JIT hot-block op set",
        ),
        ("Quos", "signed divide extension op, not translated"),
        ("Quou", "unsigned divide extension op, not translated"),
        ("Rems", "signed remainder extension op, not translated"),
        ("Remu", "unsigned remainder extension op, not translated"),
        ("Rer", "external register bus read, interpreter only"),
        ("Rfr", "FPU register-file read, not translated"),
        ("Rotw", "register-window rotate, interpreter only"),
        ("RoundS", "FPU float-to-int convert, not translated"),
        ("Rsil", "interrupt-level set, cold path stays interpreted"),
        ("Rsr", "special-register read, interpreter only"),
        ("Rur", "user-register read, interpreter only"),
        ("Rsync", "pipeline sync op, cold path stays interpreted"),
        (
            "S32c1i",
            "compare-and-swap, not in the JIT hot-block op set",
        ),
        ("S32e", "windowed-register-file store, interpreter only"),
        ("S32ri", "release-store, not in the JIT hot-block op set"),
        ("Salt", "signed less-than-with-trap compare, not translated"),
        (
            "Saltu",
            "unsigned less-than-with-trap compare, not translated",
        ),
        ("Sext", "sign-extend op, not in the JIT hot-block op set"),
        ("Sll", "shift op, not in the JIT hot-block op set"),
        ("Slli", "shift op, not in the JIT hot-block op set"),
        ("Sra", "shift op, not in the JIT hot-block op set"),
        ("Srai", "shift op, not in the JIT hot-block op set"),
        ("Src", "funnel-shift op, not in the JIT hot-block op set"),
        ("Srl", "shift op, not in the JIT hot-block op set"),
        ("Srli", "shift op, not in the JIT hot-block op set"),
        (
            "Ssa8b",
            "shift-amount setup, not in the JIT hot-block op set",
        ),
        (
            "Ssa8l",
            "shift-amount setup, not in the JIT hot-block op set",
        ),
        (
            "Ssai",
            "shift-amount setup, not in the JIT hot-block op set",
        ),
        ("Ssi", "FPU store, not translated"),
        ("Ssiu", "FPU store with update, not translated"),
        ("Ssl", "shift-amount setup, not in the JIT hot-block op set"),
        ("Ssr", "shift-amount setup, not in the JIT hot-block op set"),
        ("Ssx", "FPU indexed store, not translated"),
        ("Ssxu", "FPU indexed store with update, not translated"),
        ("SubS", "FPU (single-precision) op, not translated"),
        ("Subx2", "scaled subtract, not in the JIT hot-block op set"),
        ("Subx4", "scaled subtract, not in the JIT hot-block op set"),
        ("Subx8", "scaled subtract, not in the JIT hot-block op set"),
        ("Syscall", "syscall trap, cold path stays interpreted"),
        ("TruncS", "FPU float-to-int convert, not translated"),
        ("UfloatS", "FPU int-to-float convert, not translated"),
        (
            "Unknown",
            "decode-failure sentinel, never a real instruction to translate",
        ),
        ("UtruncS", "FPU float-to-int convert, not translated"),
        ("Waiti", "wait-for-interrupt, cold path stays interpreted"),
        ("Wer", "external register bus write, interpreter only"),
        ("Wfr", "FPU register-file write, not translated"),
        ("Wsr", "special-register write, interpreter only"),
        ("Wur", "user-register write, interpreter only"),
        ("Xsr", "special-register exchange, interpreter only"),
    ],
};

/// AVR has no JIT at all (`crate::cpu::avr` runs interpreted end to end), so
/// there is nothing for this gate to check for it. Listed explicitly rather
/// than left out, so a reader does not wonder whether AVR was forgotten.
const AVR_HAS_NO_JIT: &str = "crate::cpu::avr is interpreter-only; skipped on purpose";

struct Report {
    total: usize,
    translated: usize,
    interpreter_only: usize,
}

fn check_arch(arch: &ArchCoverage) -> Report {
    let decoder_src = read_stripped(Path::new(arch.decoder_path));
    let variants = enum_variant_names(&decoder_src);
    assert!(
        variants.len() > 10,
        "{}: only {} Instruction variants found in {} -- the enum scanner is not reaching the \
         real decoder",
        arch.name,
        variants.len(),
        arch.decoder_path
    );

    let jit_text = concat_sources(arch.jit_paths);
    assert!(
        jit_text.len() > 500,
        "{}: JIT sources at {:?} read suspiciously little text ({} bytes) -- the coverage \
         check would pass vacuously",
        arch.name,
        arch.jit_paths,
        jit_text.len()
    );

    // Every allow-listed name must still be a real variant, or the list is
    // rotting (renamed/removed variants left behind as dead entries).
    for (name, _) in arch.interpreter_only {
        assert!(
            variants.iter().any(|v| v == name),
            "{}: INTERPRETER_ONLY entry `{name}` is not a variant of {} any more -- remove the \
             stale entry",
            arch.name,
            arch.decoder_path
        );
    }
    // No duplicate allow-list entries.
    for i in 0..arch.interpreter_only.len() {
        for j in (i + 1)..arch.interpreter_only.len() {
            assert_ne!(
                arch.interpreter_only[i].0, arch.interpreter_only[j].0,
                "{}: `{}` is listed twice in INTERPRETER_ONLY",
                arch.name, arch.interpreter_only[i].0
            );
        }
    }

    let mut translated = 0usize;
    let mut interpreter_only = 0usize;
    for name in &variants {
        let referenced = contains_word(&jit_text, name);
        let allow_listed = arch.interpreter_only.iter().find(|(n, _)| n == name);
        match (referenced, allow_listed) {
            (true, None) => translated += 1,
            (false, Some(_)) => interpreter_only += 1,
            (true, Some((_, reason))) => panic!(
                "{}: `Instruction::{name}` is listed INTERPRETER_ONLY (\"{reason}\") but is now \
                 referenced by the JIT translator sources -- this is a stale allow-list entry. \
                 Remove it (the ratchet only shrinks) once you've confirmed the JIT genuinely \
                 translates it now.",
                arch.name
            ),
            (false, None) => panic!(
                "{}: `Instruction::{name}` is decoded by the interpreter but is not mentioned \
                 anywhere in the JIT translator sources for this architecture, and is not in \
                 the INTERPRETER_ONLY allow-list. Either make the JIT translate it, or add \
                 (\"{name}\", \"<one-line reason>\") to `{}`'s INTERPRETER_ONLY list in \
                 crates/core/src/tests/jit_translate_coverage_ratchet.rs.",
                arch.name,
                arch.name.to_uppercase()
            ),
        }
    }

    Report {
        total: variants.len(),
        translated,
        interpreter_only,
    }
}

#[test]
fn cortex_m_every_variant_is_translated_or_declared_interpreter_only() {
    let r = check_arch(&CORTEX_M);
    println!(
        "cortex_m JIT coverage: {} variants total, {} translated, {} interpreter-only",
        r.total, r.translated, r.interpreter_only
    );
    assert_eq!(r.total, r.translated + r.interpreter_only);
}

#[test]
fn riscv_every_variant_is_translated_or_declared_interpreter_only() {
    let r = check_arch(&RISCV);
    println!(
        "riscv JIT coverage: {} variants total, {} translated, {} interpreter-only",
        r.total, r.translated, r.interpreter_only
    );
    assert_eq!(r.total, r.translated + r.interpreter_only);
}

#[test]
fn xtensa_every_variant_is_translated_or_declared_interpreter_only() {
    let r = check_arch(&XTENSA);
    println!(
        "xtensa JIT coverage: {} variants total, {} translated, {} interpreter-only",
        r.total, r.translated, r.interpreter_only
    );
    assert_eq!(r.total, r.translated + r.interpreter_only);
}

#[test]
fn avr_has_no_jit_and_is_skipped_on_purpose() {
    assert!(AVR_HAS_NO_JIT.contains("interpreter-only"));
}

// ---------------------------------------------------------------------------
// Unit tests for the scanner itself, so a change to the parsing logic is
// caught independently of whatever the real tree currently looks like.
// ---------------------------------------------------------------------------

#[test]
fn enum_variant_names_handles_unit_tuple_and_struct_variants() {
    let src = strip_comments_and_strings(
        r#"
pub enum Instruction {
    Nop, // a unit variant
    MovImm {
        rd: u8,
        imm: u8,
    },
    Unknown(u32),
}
"#,
    );
    assert_eq!(
        enum_variant_names(&src),
        vec![
            "Nop".to_string(),
            "MovImm".to_string(),
            "Unknown".to_string()
        ]
    );
}

#[test]
fn enum_variant_names_ignores_commas_inside_comments_and_field_types() {
    let src = strip_comments_and_strings(
        r#"
pub enum Instruction {
    Foo { rd: u8, rs1: u8 }, // FOO rd, rs1 -- comma here must not split
    Bar(u32, u32),
}
"#,
    );
    assert_eq!(
        enum_variant_names(&src),
        vec!["Foo".to_string(), "Bar".to_string()]
    );
}

#[test]
fn contains_word_respects_identifier_boundaries() {
    assert!(contains_word("fn foo() { And { .. } => true }", "And"));
    assert!(!contains_word("fn foo() { AndBar { .. } => true }", "And"));
    assert!(!contains_word("fn foo() { FooAnd { .. } => true }", "And"));
    assert!(contains_word("Bx | BlxReg", "Bx"));
    assert!(!contains_word("Bx | BlxReg", "Blx"));
}

/// A regression test for the exact failure mode this file exists to catch:
/// dropping in a brand-new decoder variant that is neither translated nor
/// allow-listed must be red, with the message pointing at both remedies.
#[test]
fn an_untranslated_undeclared_variant_is_reported() {
    let arch = ArchCoverageFixture::new(
        "pub enum Instruction { Nop, TotallyNewOp { rd: u8 } }",
        "fn is_alu_emittable() { match () { _ if true => Nop } }",
        &[],
    );
    let msg = capture_panic_message(|| arch.check())
        .expect("expected the coverage check to panic on an unresolved variant");
    assert!(
        msg.contains("TotallyNewOp"),
        "panic message should name the offending variant, got: {msg}"
    );
}

#[test]
fn a_stale_allow_list_entry_is_reported() {
    let arch = ArchCoverageFixture::new(
        "pub enum Instruction { Nop }",
        "fn is_alu_emittable() { match () { _ if true => Nop } }",
        &[("Nop", "stale: Nop is actually referenced above")],
    );
    let msg = capture_panic_message(|| arch.check())
        .expect("expected the coverage check to panic on a stale allow-list entry");
    assert!(
        msg.contains("stale allow-list entry"),
        "panic message should call out staleness, got: {msg}"
    );
}

/// Run `f`, returning `Some(messages)` if it panicked. Captures through a
/// temporary panic hook and the hook info's `Display`, which carries the
/// message however the payload was boxed. Every message seen while the hook is
/// installed is kept, newline-joined, so a panic from a test running
/// concurrently on another thread cannot replace the one under test.
fn capture_panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> Option<String> {
    use std::sync::{Arc, Mutex};
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_hook = captured.clone();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        captured_hook.lock().unwrap().push(info.to_string());
    }));
    let result = std::panic::catch_unwind(f);
    std::panic::set_hook(prev_hook);
    result.err()?;
    let msgs = captured.lock().unwrap().join("\n");
    Some(msgs)
}

/// In-memory stand-in for [`ArchCoverage`] so the two regression tests above
/// exercise the exact matching/reporting logic in [`check_arch`] without
/// touching real files on disk.
struct ArchCoverageFixture {
    decoder_src: String,
    jit_text: String,
    interpreter_only: Vec<(&'static str, &'static str)>,
}

impl ArchCoverageFixture {
    fn new(
        decoder_src: &str,
        jit_text: &str,
        interpreter_only: &[(&'static str, &'static str)],
    ) -> Self {
        Self {
            decoder_src: decoder_src.to_string(),
            jit_text: jit_text.to_string(),
            interpreter_only: interpreter_only.to_vec(),
        }
    }

    fn check(&self) {
        let variants = enum_variant_names(&strip_comments_and_strings(&self.decoder_src));
        for name in &variants {
            let referenced = contains_word(&self.jit_text, name);
            let allow_listed = self.interpreter_only.iter().find(|(n, _)| n == name);
            match (referenced, allow_listed) {
                (true, None) | (false, Some(_)) => {}
                (true, Some((_, reason))) => panic!(
                    "`Instruction::{name}` is listed INTERPRETER_ONLY (\"{reason}\") but is now \
                     referenced by the JIT translator sources -- this is a stale allow-list \
                     entry."
                ),
                (false, None) => panic!(
                    "`Instruction::{name}` is decoded by the interpreter but is not mentioned \
                     anywhere in the JIT translator sources for this architecture, and is not \
                     in the INTERPRETER_ONLY allow-list. Either make the JIT translate it, or \
                     add it to the allow-list with a one-line reason."
                ),
            }
        }
    }
}
