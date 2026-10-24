// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Static branch discovery on committed firmware, ratcheted against
//! `llvm-objdump`.
//!
//! The expected counts were produced by disassembling each ELF with
//! llvm-objdump 18 and counting every conditional branch (`b<cond>`, `cbz`,
//! `cbnz`; `beq bne blt bge bltu bgeu c.beqz c.bnez`) with two different
//! outcomes. The same comparison over all 81 committed ELFs that carry line
//! info found every such branch with no extras and no wrong targets; the
//! files below pin the interesting cases.

use labwired_loader::branch_sites::{decode_with_code_ranges, BranchIsa, BranchSites};
use labwired_loader::SymbolProvider;
use std::path::PathBuf;

fn sites(rel: &str) -> BranchSites {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let symbols = SymbolProvider::new(&path).unwrap();
    decode_with_code_ranges(&bytes, symbols.line_sequences())
}

fn has(s: &BranchSites, addr: u64, target: u64) -> bool {
    s.sites.iter().any(|b| b.addr == addr && b.target == target)
}

/// Bare-metal C on Cortex-M33: literal pools are marked `$d` and skipped.
#[test]
fn thumb_c_firmware() {
    let s = sites("tests/fixtures/nrf54l15-smart-ring.elf");
    assert_eq!(s.isa, Some(BranchIsa::Thumb));
    assert_eq!(s.sites.len(), 16);
}

/// Zephyr on Cortex-M0+: objdump lists 473 conditional branches; one is
/// `beq` to the very next instruction in `z_arm_svc`, which has a single
/// outcome and is not a branch to cover.
#[test]
fn thumb_zephyr_kernel() {
    let s = sites("tests/fixtures/kw41z-zephyr-hello.elf");
    assert_eq!(s.sites.len(), 472);
    assert!(
        !s.sites.iter().any(|b| b.target == b.fallthrough()),
        "no single-outcome branches"
    );
}

/// RISC-V with hand-written startup assembly: three of the branches live in
/// `_start`, which has no sized symbol, and are reached through the `$x`
/// mapping symbol.
#[test]
fn riscv_startup_assembly() {
    let s = sites("tests/fixtures/riscv-ci-fixture.elf");
    assert_eq!(s.isa, Some(BranchIsa::RiscV));
    assert_eq!(s.sites.len(), 14);
    for addr in [0x8000_0094, 0x8000_00a4, 0x8000_00b8] {
        assert!(s.sites.iter().any(|b| b.addr == addr), "{addr:#x}");
    }
}

/// ESP32-C3 Arduino core (RV32IMC, compressed branches, 6 469 sites). The
/// FreeRTOS port's `portasm.S` has line info but neither sized symbols nor
/// mapping symbols; only the DWARF line sequences reach it.
#[test]
fn riscv_arduino_core_with_rtos_assembly() {
    let s = sites("tests/fixtures/source-debug/esp32c3-arduino.elf");
    assert_eq!(s.sites.len(), 6469);
    assert!(has(&s, 0x4038_560a, 0x4038_5636), "beq in rtos_int_enter");
    assert!(has(&s, 0x4038_566c, 0x4038_568c), "beq in rtos_int_exit");
    assert!(
        s.sites.iter().any(|b| b.len == 2),
        "c.beqz / c.bnez are decoded"
    );
}

/// A linker that garbage-collects functions leaves their line-table
/// sequences behind at address 0. In this Rust/Cortex-M0+ build the
/// discarded `compiler-builtins` sequences overlap real code from 0x0 to
/// 0x77e; none of them may claim a branch of the code that is there.
#[test]
fn discarded_functions_do_not_claim_live_branches() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/kw41z-lcd-activity.elf");
    let bytes = std::fs::read(&path).unwrap();
    let symbols = SymbolProvider::new(&path).unwrap();
    let s = decode_with_code_ranges(&bytes, symbols.line_sequences());
    assert_eq!(s.sites.len(), 440);

    let mut in_functions = 0;
    for b in &s.sites {
        let Some((_, _, name)) = symbols
            .functions()
            .iter()
            .find(|(a, n, _)| b.addr >= *a && b.addr < a + n)
        else {
            continue;
        };
        let Some(row) = symbols.line_at(b.addr) else {
            continue;
        };
        in_functions += 1;
        // compiler-builtins also exports C-ABI names (`__aeabi_memclr4`).
        let builtins = name.contains("compiler_builtins") || name.starts_with("__");
        if !builtins {
            assert!(
                !row.path.contains("compiler-builtins"),
                "{:#x} in {name} attributed to {}:{}",
                b.addr,
                row.path,
                row.line
            );
        }
    }
    assert_eq!(
        in_functions, 436,
        "every branch inside a function has a line"
    );

    // `core::fmt::num` at 0xfc..0x204 keeps its own lines.
    let row = symbols.line_at(0x10c).unwrap();
    assert!(row.path.ends_with("core/src/fmt/num.rs"), "{}", row.path);
    // `Reset` (assembly, no line info) is only covered by discarded
    // sequences, so it has no line rather than a wrong one.
    assert!(symbols.line_at(0xcc).is_none());
}
