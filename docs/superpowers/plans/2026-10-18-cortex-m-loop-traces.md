# Cortex-M Loop-Trace JIT Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the browser Cortex-M JIT profitable by compiling counted ALU loops into traces that run many iterations per JS boundary, with machine-state equality to the interpreter proven by a native differential gate.

**Architecture:** The shared Thumb frontend (`crates/core/src/cpu/jit_framework/cortex_m/`) gains a trace emission: a block whose conditional backedge targets its own entry and whose body is pure ALU (leaf calls inlined one level) emits a single wasm `loop` that runs `k` iterations internally, where `k = min(iterations-cap staged by the runtime, natural exit)`. Both runtimes (native wasmtime in `exec.rs`, browser `js_sys::WebAssembly` in `crates/wasm/src/jit_browser.rs`) consume the identical emitted bytes — that byte-for-byte sharing is the framework's stated contract. `BlockPlan` gains `iter_instr_count: Option<u32>`; the runtime stages a cap into the register-staging memory at `ITERS_CAP_SLOT` before the call and reads back how many iterations ran from `ITERS_DONE_SLOT` after it, then charges `k * n` cycles exactly as it would have charged `k` separate block entries.

**Tech Stack:** Rust 2021, hand-rolled wasm binary encoder (no new dependencies), wasmtime (native differential), wasm-bindgen/js_sys (browser adapter).

---

## Context the executor MUST read first

- `crates/core/src/cpu/jit_framework/cortex_m/emit.rs` — `emit_block`, `Body::emit_instruction`, `Body::emit_terminator` (see the `BranchCond` arm at ~line 2841), `emit_prologue`/`emit_epilogue` (~2977), the slot constants at the top (NEXT_PC_SLOT 64, FAULT_PC_SLOT 68, FAULT_RETIRED_SLOT 72, RES_FLAG_SLOT 76, IT_STATE_SLOT 80), `LOCAL_COUNT` (= 20 today), `build_module`/`build_module_ram_host`.
- `crates/core/src/cpu/jit_framework/cortex_m/mod.rs` — `CortexMFrontend::translate_block_thumb`, `BlockPlan` construction.
- `crates/core/src/cpu/jit_framework/frontend.rs` — `BlockPlan` doc contract.
- `crates/core/src/cpu/jit_framework/cortex_m/exec.rs` — native runtime: `CompiledBlock`, `REG_SYNC_BYTES`, `EngineStats`, `CortexMJitEngine`.
- `crates/core/src/cpu/cortex_m.rs` — `run_jit_loop` (~line 2439), `jit_gate_allows` (~2403), `block_would_cross_irq` (~2389), `CORTEX_M_JIT_HOT_THRESHOLD = 50` (~2383).
- `crates/wasm/src/jit_browser.rs` — browser runtime: `CortexMBrowserBlock` (~901), its `compile` (~917) and `run` (~1038), `try_browser_cortex_m_jit_step` (~1217), `run_browser_cortex_m_jit_window` (~1361), `BrowserJitCache` (~425).
- `crates/core/tests/cortex_m_jit_nrf_differential.rs` — the test template (hand-assembled Thumb halfwords written into `bus.flash.data`, `snapshot_state` + `compare_memory` equality).

**Out of scope (V1, deliberate):** traces never contain memory ops, VFP, IT blocks, or `BlxReg`. Memory loops are V2 — they need fault accounting and RAM-host import redesign (see `rp2040-jit-investigation.json` in `~/outputs/kernelcad-drawing-robot/raw-labwired/`).

## House rules for every commit / PR in this plan

- Commit as `Andrii Shylenko <14119286+w1ne@users.noreply.github.com>`; **never** a `Co-Authored-By: Claude` trailer or "Generated with" footer.
- Work in a worktree, never rebase; merge `origin/main` into the branch when behind.
- Stage everything (`git add -A`) before each commit; commit message style follows recent log (`perf(wasm): ...`, body explains why + evidence).
- PRs: own branch, wait for the full gate set, verify the head SHA and that the load-bearing steps RAN (not skipped), then merge with `gh pr merge <n> --merge` (no `--admin`, no `--delete-branch`).
- Every build that compiles core locally: `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1`, `CARGO_TARGET_DIR=/Volumes/LabWired/.cargo-targets/<name>`; long commands foreground.
- Delete this plan file in the final PR of the series.

---

### Task 1: Config flag + failing differential test

**Files:**
- Modify: `crates/core/src/config.rs` (add field near `cortex_m_jit_enabled`, line ~56)
- Create: `crates/core/tests/cortex_m_loop_trace_differential.rs`

- [ ] **Step 1: Add the config flag**

In `crates/core/src/config.rs`, after `cortex_m_jit_enabled`:

```rust
    /// Cortex-M loop TRACES: a counted ALU loop whose conditional backedge
    /// targets its own entry compiles to a wasm `loop` that runs many
    /// iterations per dispatch. Only consulted when
    /// [`Self::cortex_m_jit_enabled`] is true. Default off; the native
    /// differential (`tests/cortex_m_loop_trace_differential.rs`) and the
    /// browser adapter opt in.
    pub cortex_m_jit_loop_traces: bool,
```

and in the `Default` impl next to `cortex_m_jit_enabled: false`:

```rust
            cortex_m_jit_loop_traces: false,
```

- [ ] **Step 2: Write the failing differential test**

Create `crates/core/tests/cortex_m_loop_trace_differential.rs`:

```rust
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Loop traces must retire exactly the cycles, and end in exactly the
//! machine state, the interpreter produces for the same period.
//!
//! The fixture is the shipped RP2040 demo's hot shape, hand-assembled:
//! a counted loop whose body is pure ALU (`nop; adds; subs; bne head`).
//! With the trace on, one dispatch retires many iterations; with it off,
//! one dispatch per iteration. Both must land on the same pc, flags,
//! registers, memory and cycle count.

#![cfg(all(feature = "jit", feature = "event-scheduler"))]

use labwired_core::bus::{SystemBus, RECOMMENDED_TICK_INTERVAL};
use labwired_core::cpu::jit_framework::cortex_m::snapshot_state;
use labwired_core::cpu::jit_framework::differential::compare_memory;
use labwired_core::cpu::CortexM;
use labwired_core::{Machine};

fn h(bytes: &mut Vec<u8>, half: u16) {
    bytes.extend_from_slice(&half.to_le_bytes());
}

/// The RP2040 demo's hot loop, without the call:
///   0x00 movs r4, #1000   ; r4 = iter counter
///   0x02 movs r0, #0      ; r0 = accumulator
///   0x04 nop              ; <- loop head (traced entry)
///   0x06 adds r0, #1
///   0x08 subs r4, #1      ; sets Z on the last iteration
///   0x0A bne  0x04        ; backedge to the head
///   0x0C movs r1, #1      ; done marker
///   0x0E b .              ; park
fn counted_loop_machine(jit: bool, traces: bool) -> Machine<CortexM> {
    let mut prog = Vec::new();
    h(&mut prog, 0x24FA); // movs r4, #250 (0xFA) — 250 iterations
    h(&mut prog, 0x2000); // movs r0, #0
    h(&mut prog, 0xBF00); // nop              @0x04 head
    h(&mut prog, 0x3001); // adds r0, #1
    h(&mut prog, 0x3C01); // subs r4, #1
    h(&mut prog, 0xD1FB); // bne  0x04  (imm8 = -10: 0x0A+4-10)
    h(&mut prog, 0x2101); // movs r1, #1
    h(&mut prog, 0xE7FE); // b .
    let mut bus = SystemBus::new();
    let n = prog.len();
    bus.flash.data[..n].copy_from_slice(&prog);
    bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    let mut cpu = CortexM::new();
    cpu.pc = 0;
    cpu.sp = 0x2000_1000;
    cpu.xpsr = 0x0100_0000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.cortex_m_jit_enabled = jit;
    machine.config.cortex_m_jit_loop_traces = traces;
    machine.bus.config.cortex_m_jit_enabled = jit;
    machine.bus.legacy_walk_disabled = true;
    machine
}

#[test]
fn a_traced_counted_loop_retires_the_same_machine() {
    let cycles = 20_000; // >> hot threshold (50 arrivals) and >> the loop
    let mut off = counted_loop_machine(true, false);
    let mut on = counted_loop_machine(true, true);
    off.run(Some(cycles)).unwrap();
    on.run(Some(cycles)).unwrap();

    assert_eq!(
        snapshot_state(&off.cpu),
        snapshot_state(&on.cpu),
        "the trace changed the machine state"
    );
    assert_eq!(
        off.total_cycles, on.total_cycles,
        "the trace changed the retired cycle count"
    );
    assert_eq!(on.cpu.read_reg(0), 250 & 0xFF, "accumulator ran the loop once");
    assert_eq!(on.cpu.read_reg(1), 1, "the loop exited and set the marker");
    if let Some(d) = compare_memory(off.total_cycles, off.cpu.pc, &off.bus, &on.bus) {
        panic!("trace RAM mismatch: {d:?}");
    }

    // Non-vacuous: the trace path must have actually retired iterations.
    let stats = on.cpu.jit_stats().expect("jit engine present");
    assert!(stats.trace_iters > 0, "the loop trace never ran: {stats:?}");
    let off_stats = off.cpu.jit_stats().expect("jit engine present");
    assert_eq!(off_stats.trace_iters, 0, "traces off must not trace");
}

#[test]
fn a_trace_stops_at_a_partial_window_and_resumes_at_the_head() {
    // Budget smaller than one natural run (250 iters * 4 instrs = 1000
    // cycles): the trace must stop mid-loop, report its iterations, and
    // the next dispatch must resume at the head — final state equal.
    let mut off = counted_loop_machine(true, false);
    let mut on = counted_loop_machine(true, true);
    for _ in 0..40 {
        off.run(Some(64)).unwrap();
        on.run(Some(64)).unwrap();
        assert_eq!(off.cpu.pc, on.cpu.pc, "resume pc diverged");
    }
    assert_eq!(snapshot_state(&off.cpu), snapshot_state(&on.cpu));
    assert_eq!(off.total_cycles, on.total_cycles);
}
```

- [ ] **Step 3: Run it — expect a compile failure**

Run: `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 CARGO_TARGET_DIR=/Volumes/LabWired/.cargo-targets/core-lt cargo test -p labwired-core --features jit,event-scheduler --test cortex_m_loop_trace_differential`
Expected: `error[E0609]: no field cortex_m_jit_loop_traces` (until Task 1 Step 1 landed) — after Step 1 it compiles and the test FAILS with `the loop trace never ran` because `trace_iters` does not exist yet (`E0609` on the field access in the last asserts is the expected signal).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "test(cortex-m): failing loop-trace differential (fixture + non-vacuous stats)"
```

---

### Task 2: wasm encoder — `block`, `loop`, `br`, `br_if`

**Files:**
- Modify: `crates/core/src/cpu/jit_framework/riscv/wasm_encode.rs` (the `op` module, ~line 72-120)

- [ ] **Step 1: Add the four opcode constants**

In `pub mod op` next to `IF`/`ELSE`/`END`:

```rust
    pub const BLOCK: u8 = 0x02;
    pub const LOOP: u8 = 0x03;
    pub const BR: u8 = 0x0c;
    pub const BR_IF: u8 = 0x0d;
```

- [ ] **Step 2: Compile check**

Run: `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 cargo check -p labwired-core --features jit`
Expected: `Finished`.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(jit): wasm encoder gains block/loop/br/br_if opcodes"
```

---

### Task 3: `emit_loop_trace` in the Thumb emitter

**Files:**
- Modify: `crates/core/src/cpu/jit_framework/cortex_m/emit.rs`

- [ ] **Step 1: Slots and locals**

Below `IT_STATE_SLOT`:

```rust
/// Iterations cap staged by the runtime before a loop trace runs, and the
/// iterations actually retired, written back by the trace. Both live in the
/// same register-staging memory as the slots above; `THUMB_REG_BYTES` must
/// cover `ITERS_DONE_SLOT + 4`.
pub const ITERS_CAP_SLOT: u32 = 84;
pub const ITERS_DONE_SLOT: u32 = 88;
```

Change `const LOCAL_COUNT: u32 = 20;` to `22` and add:

```rust
const CAP_LOCAL: u32 = 20;
const ITERS_LOCAL: u32 = 21;
```

Then set `const THUMB_REG_BYTES: usize` (in `crates/wasm/src/jit_browser.rs:885`) so the read-back covers the new tail: check today's value and make it `ITERS_DONE_SLOT as usize + 4` rounded to 96 — it already is 96, so just add a comment `// covers ITERS_DONE_SLOT (88..92)`.

- [ ] **Step 2: Write the emission**

Add after `emit_block` (all helpers it calls — `walk_ops`, `is_emittable`, `Body::emit_instruction`, `push_condition`, `local_get/set/tee`, `i32_const`, `next_pc_const`, `enc::sleb/uleb` — already exist in this file):

```rust
/// Try to emit a loop TRACE at `pc`.
///
/// Qualifying shape (V1, deliberately narrow):
///   * straight-line pure-ALU ops from `pc` (no memory, no VFP, no IT);
///   * any `Bl` in that straight line is INLINED one level: its target
///     must decode, inside the same code view, to pure-ALU ops ending in
///     `Bx { rm: 14 }` (a leaf return), with no further calls;
///   * the terminator is `BranchCond { cond, offset }` whose target is
///     exactly `pc` — the conditional backedge to the entry;
///   * at least one body op sets flags (otherwise the branch decision
///     cannot change and the loop is a park the poll-loop skip owns).
///
/// The emitted body runs `min(cap, natural exit)` iterations internally,
/// where `cap` is read from [`ITERS_CAP_SLOT`]; it writes the number of
/// iterations actually run to [`ITERS_DONE_SLOT`] and exits with
/// `WIRE_CHAIN_DYNAMIC`, `NEXT_PC_SLOT` = head pc on a cap exit and
/// fall-through on a natural exit.
pub fn emit_loop_trace(pc: Pc, code: &CodeView<'_>) -> Option<EmittedBlock> {
    let ops = walk_ops(pc, code, false);
    if ops.is_empty() {
        return None;
    }
    // Walk the straight line, inlining Bl leaves one level deep.
    let mut body = Body::default();
    let mut per_iter: u32 = 0;
    let mut sets_flags = false;
    for aop in &ops {
        match &aop.inst {
            Instruction::Bl { .. } => {
                // Inline a leaf callee: ops ending in Bx { rm: 14 }.
                let ret = aop.pc + 4;
                body.i32_const((ret | 1) as i32);
                body.write(14); // LR, as the standalone Bl terminator does
                per_iter += 1;
                let callee_pc = ret.wrapping_add(inst_offset_for(&aop.inst) as u32); // see helper below
                let callee_ops = walk_ops(callee_pc, code, false);
                let mut saw_return = false;
                for cop in &callee_ops {
                    if !is_alu_emittable(&cop.inst) {
                        return None;
                    }
                    body.emit_instruction(cop.pc, &cop.inst, code);
                    per_iter += 1;
                }
                // The leaf's return instruction.
                let ret_pc = callee_pc + callee_ops.iter().map(|o| inst_len_of(o.pc, code)).sum::<u64>();
                match decode_at(ret_pc, code) {
                    Some((Instruction::Bx { rm: 14 }, _)) => saw_return = true,
                    _ => return None,
                }
                if !saw_return {
                    return None;
                }
                per_iter += 1;
            }
            inst if is_alu_emittable(inst) => {
                body.emit_instruction(aop.pc, inst, code);
                per_iter += 1;
                sets_flags |= sets_flags_of(inst);
            }
            _ => return None,
        }
    }
    let prefix_end = pc + ops.iter().map(|o| inst_len_of(o.pc, code)).sum::<u64>();
    let Some((Instruction::BranchCond { cond, offset }, tlen)) = decode_at(prefix_end, code) else {
        return None;
    };
    if prefix_end.wrapping_add(4).wrapping_add(offset as u64) != pc || !sets_flags {
        return None;
    }
    let per_iter = per_iter + 1; // + the BranchCond itself
    if per_iter > MAX_TRACE_INSTRS {
        return None;
    }

    let mut expr = Vec::with_capacity(body.buf.len() + 512);
    body.emit_prologue(&mut expr);
    // Staged iteration cap.
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, ITERS_CAP_SLOT as i64);
    expr.push(op::I32_LOAD);
    enc::uleb(&mut expr, 2);
    enc::uleb(&mut expr, 0);
    expr.push(op::LOCAL_SET);
    enc::uleb(&mut expr, CAP_LOCAL as u64);
    expr.push(op::LOCAL_GET);
    enc::uleb(&mut expr, CAP_LOCAL as u64);
    expr.push(op::LOCAL_SET);
    enc::uleb(&mut expr, ITERS_LOCAL as u64);

    expr.push(op::BLOCK);
    expr.push(op::T_I32); // block type: void is 0x40; we only branch, so void
    // NOTE: use void block type — see op::T_VOID below; if the encoder has
    // no T_VOID constant, push 0x40 literally.
    expr.push(op::LOOP);
    expr.push(0x40); // void loop type
    expr.extend_from_slice(&body.buf);
    // Natural exit: continue while `cond` holds, exit when it does not.
    body.push_condition(cond);
    expr.push(op::I32_EQZ);
    expr.push(op::BR_IF);
    enc::uleb(&mut expr, 1); // leave the loop, exit the block
    // Bookkeeping: iters -= 1; continue while non-zero.
    expr.push(op::LOCAL_GET);
    enc::uleb(&mut expr, ITERS_LOCAL as u64);
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, 1);
    expr.push(op::I32_SUB);
    expr.push(op::LOCAL_TEE);
    enc::uleb(&mut expr, ITERS_LOCAL as u64);
    expr.push(op::BR_IF);
    enc::uleb(&mut expr, 0); // loop again
    expr.push(op::END); // loop
    expr.push(op::END); // block
    // Iterations actually run = cap - remaining.
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, ITERS_DONE_SLOT as i64);
    expr.push(op::LOCAL_GET);
    enc::uleb(&mut expr, CAP_LOCAL as u64);
    expr.push(op::LOCAL_GET);
    enc::uleb(&mut expr, ITERS_LOCAL as u64);
    expr.push(op::I32_SUB);
    expr.push(op::I32_STORE);
    enc::uleb(&mut expr, 2);
    enc::uleb(&mut expr, 0);
    // Next pc: flags still hold the last iteration's decision (nothing in
    // this epilogue touches them), so re-test `cond` to distinguish a
    // natural exit (fall through) from a cap exit (stay at the head).
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, NEXT_PC_SLOT as i64);
    body.push_condition(cond);
    expr.push(op::IF);
    expr.push(op::T_I32);
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, pc as i64); // cap exit: resume at the head
    expr.push(op::ELSE);
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, (prefix_end + tlen) as i64); // natural: fall through
    expr.push(op::END);
    expr.push(op::I32_STORE);
    enc::uleb(&mut expr, 2);
    enc::uleb(&mut expr, 0);
    body.emit_epilogue(&mut expr);
    expr.push(op::I32_CONST);
    enc::sleb(&mut expr, WIRE_CHAIN_DYNAMIC as i64);

    let code_bytes = build_module(LOCAL_COUNT, 1, &expr);
    Some(EmittedBlock {
        code: code_bytes,
        end_pc: prefix_end + tlen,
        instr_count: per_iter,
        iter_instr_count: Some(per_iter),
        exits: vec![ExitEdge {
            wire_code: WIRE_CHAIN_DYNAMIC,
            reason: BailReason::PartialBlock,
        }],
        binding: None,
    })
}

/// Body instructions that set APSR flags (the same set the interpreter
/// writes flags for; kept conservative — a false positive only widens the
/// trace eligibility, never breaks semantics because the loop still exits
/// on its own condition).
fn sets_flags_of(inst: &Instruction) -> bool {
    use Instruction::*;
    matches!(
        inst,
        AddReg { .. } | AddImm3 { .. } | AddImm8 { .. } | SubReg { .. } | SubImm3 { .. }
            | SubImm8 { .. } | CmpImm { .. } | CmpReg { .. } | Cmn { .. } | Tst { .. }
            | And { .. } | Orr { .. } | Eor { .. } | Mvn { .. } | Lsl { .. } | Lsr { .. }
            | Asr { .. } | Adc { .. } | Sbc { .. } | Rsbs { .. } | Mul { .. } | Uxtb { .. }
            | Uxth { .. } | Sxtb { .. } | Sxth { .. }
    )
}
```

Implementation notes the executor will need:
- `EmittedBlock` gains `pub iter_instr_count: Option<u32>`; set `None` in the existing `emit_block` return.
- `Bl`'s offset: read the decoder's `Instruction::Bl { offset }` field — the helper `inst_offset_for` above is a placeholder: use the actual `offset` out of the matched variant (`Instruction::Bl { offset }`), target = `aop.pc + 4 + offset`. The standalone `emit_terminator` `Bl` arm at ~2875 shows the exact arithmetic.
- `walk_ops(pc, code, mem_ok)` is the existing straight-line walker (used by `emit_block` with `window.is_some()`); its stopping rules stay untouched.
- `MAX_TRACE_INSTRS` — add `const MAX_TRACE_INSTRS: u32 = 512;` next to `MAX_BLOCK_INSTRS`.
- Confirm the exact block-type byte for a void block: this encoder has `op::T_I32 = 0x7f`; void is `0x40`. If `op` has no constant, use the literal `0x40` in both places.

- [ ] **Step 3: Unit test the refusal set (in `emit.rs`'s test module)**

```rust
#[test]
fn a_counted_alu_loop_emits_a_trace_and_loops_with_memory_or_vfp_do_not() {
    fn h(b: &mut Vec<u8>, x: u16) { b.extend_from_slice(&x.to_le_bytes()); }
    let mut prog = Vec::new();
    h(&mut prog, 0x24FA); h(&mut prog, 0x2000); h(&mut prog, 0xBF00);
    h(&mut prog, 0x3001); h(&mut prog, 0x3C01); h(&mut prog, 0xD1FB);
    let mut code = [0u8; 4096];
    code[..prog.len()].copy_from_slice(&prog);
    let view = CodeView::new(0, &code);
    let trace = emit_loop_trace(0x04, &view).expect("trace");
    assert_eq!(trace.iter_instr_count, Some(4));
    // The same shape with a load in the body refuses.
    h(&mut prog, 0x0000);
    let mut prog2 = Vec::new();
    h(&mut prog2, 0x24FA); h(&mut prog2, 0x6800); // ldr r0,[r0,#0]
    h(&mut prog2, 0x3C01); h(&mut prog2, 0xD1FC); // subs; bne head
    let mut code2 = [0u8; 4096];
    code2[..prog2.len()].copy_from_slice(&prog2);
    assert!(emit_loop_trace(0x02, &CodeView::new(0, &code2)).is_none());
}
```

(Adjust the second fixture's offsets so the backedge targets its own head; the point is the refusal.)

- [ ] **Step 4: Run the unit test**

Run: `cargo test -p labwired-core --lib jit_framework::cortex_m::emit::`
Expected: FAIL until the emission compiles the module (wasmtime validates at instantiate in the runtime tests); PASS on the unit.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(jit): emit counted ALU loop traces in the Thumb frontend"
```

---

### Task 4: Frontend + BlockPlan plumbing

**Files:**
- Modify: `crates/core/src/cpu/jit_framework/frontend.rs` (BlockPlan)
- Modify: `crates/core/src/cpu/jit_framework/cortex_m/mod.rs`

- [ ] **Step 1: `BlockPlan.iter_instr_count`**

```rust
    /// `Some(n)` for a LOOP TRACE: the block runs `k` iterations of an
    /// `n`-instruction loop per dispatch and reports `k` through the
    /// runtime's iterations-done read-back. `None` for basic blocks, whose
    /// `instr_count` is their only cost.
    pub iter_instr_count: Option<u32>,
```

Set `iter_instr_count: None` in every existing `BlockPlan` literal (there are two in `translate_block_thumb`, one in the passthrough frontend, one in the scaffold default — `cargo check` finds them all).

- [ ] **Step 2: The traced frontend entry**

In `CortexMFrontend`, below `translate_block_thumb`:

```rust
    /// [`Self::translate_block_thumb`], but a qualifying counted ALU loop
    /// compiles to a trace FIRST; everything else falls through to the
    /// basic-block path unchanged. Only called when
    /// `SimulationConfig::cortex_m_jit_loop_traces` is set.
    pub fn translate_block_thumb_traced(
        &self,
        pc: Pc,
        code: &CodeView<'_>,
    ) -> Result<(BlockPlan, Option<MemBinding>), FrontendRefusal> {
        if !code.covers(pc) {
            return Err(FrontendRefusal::PcOutOfRange);
        }
        if let Some(blk) = emit::emit_loop_trace(pc, code) {
            let plan = BlockPlan {
                entry_pc: pc,
                end_pc: blk.end_pc,
                instr_count: blk.instr_count,
                iter_instr_count: blk.iter_instr_count,
                code: blk.code,
                exits: blk.exits,
            };
            return Ok((plan, blk.binding));
        }
        self.translate_block_thumb(pc, code)
    }
```

- [ ] **Step 3: Compile**

Run: `cargo check -p labwired-core --features jit` — expect clean.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(jit): frontend entry for loop traces; BlockPlan carries iter_instr_count"
```

---

### Task 5: Native runtime — cap staging, accounting, stats

**Files:**
- Modify: `crates/core/src/cpu/jit_framework/cortex_m/exec.rs`
- Modify: `crates/core/src/cpu/cortex_m.rs` (`run_jit_loop`)

- [ ] **Step 1: `CompiledBlock` learns the trace**

- `REG_SYNC_BYTES`: change `NEXT_PC_SLOT as usize + 4` to `emit::ITERS_DONE_SLOT as usize + 4` so the read-back includes the new slots.
- Add fields `iter_instr_count: Option<u32>` to `CompiledBlock` and `EngineStats` gains `pub trace_iters: u64, pub trace_runs: u64`.
- `CompiledBlock::run` gains a parameter `iters_cap: Option<u32>`: before `self.run.call(())`, write the cap into the block's memory at `ITERS_CAP_SLOT` (same memory write path as the register staging); after, read `ITERS_DONE_SLOT` back and return it alongside the wire code.

- [ ] **Step 2: `run_jit_loop` cap + accounting**

In `run_jit_loop` (`crates/core/src/cpu/cortex_m.rs` ~2439), where the loop currently checks `block_would_cross_irq` and runs a block:

```rust
        let per_iter = block.iter_instr_count; // None for ordinary blocks
        let cap = match per_iter {
            None => None,
            Some(n) => {
                // Never start an iteration the window or the SysTick edge
                // cannot cover: the trace charges k*n cycles at exit, and a
                // tick must not fire inside it.
                let by_budget = max_n / n;
                let by_irq = match bus.systick_ticks_until_fire() {
                    Some(h) => ((h as u32).saturating_sub(1)) / n,
                    None => u32::MAX,
                };
                Some(by_budget.min(by_irq).min(MAX_TRACE_ITERS))
            }
        };
        if matches!(cap, Some(0)) {
            return Ok(0);
        }
```

`MAX_TRACE_ITERS` — add `const MAX_TRACE_ITERS: u32 = 4096;` near `CORTEX_M_JIT_HOT_THRESHOLD` (a trace must end well inside any window; 4096 × a 4-instruction loop is 16 k cycles, far above the 1024-cycle shipped interval and small enough to stay cache-warm).

Completion accounting:

```rust
        let traced = cap.is_some();
        let (wire, done) = block.run(bus, cap)?; // done = iterations actually run
        let charged = match (per_iter, traced) {
            (Some(n), true) => {
                stats.trace_runs += 1;
                stats.trace_iters += u64::from(done);
                done * n
            }
            _ => block_instr_count,
        };
```

Then the existing `bus.systick_consume_cycles(charged)` / progress bookkeeping uses `charged` exactly where `block_instr_count` was used. The block already wrote `NEXT_PC_SLOT` (head on cap exit, fall-through on natural exit), so the existing `cpu.pc = next_pc` handling needs no change.

- [ ] **Step 3: Engine selection**

Where `step_batch_jit` calls `self.jit_engine...` / the frontend (both in `cortex_m.rs` and `exec.rs`'s engine), pick the traced entry when `config.cortex_m_jit_loop_traces` is true:

```rust
        let plan = if config.cortex_m_jit_loop_traces {
            e.frontend_mut() // or the engine's frontend field
                .translate_block_thumb_traced(pc, &view)
        } else {
            e.frontend().translate_block_thumb(pc, &view)
        };
```

(Exact accessor names per `exec.rs`; add `frontend_mut()` if missing.)

- [ ] **Step 4: Run the Task 1 differential**

Run: `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 CARGO_TARGET_DIR=/Volumes/LabWired/.cargo-targets/core-lt cargo test -p labwired-core --features jit,event-scheduler --test cortex_m_loop_trace_differential -- --nocapture`
Expected: both tests PASS; `trace_iters > 0` reported.

- [ ] **Step 5: Negative control (manual, record in the commit message)**

Temporarily invert the natural-exit branch polarity in `emit_loop_trace` (`I32_EQZ` removed → exits while the condition HOLDS), rebuild, run the differential:
Expected: FAIL with diverging state/cycles. Revert the sabotage, re-run to green, then commit.

- [ ] **Step 6: Full sweep of the neighbouring gates**

```bash
cargo test -p labwired-core --features jit,event-scheduler --test cortex_m_jit_nrf_differential
cargo test -p labwired-core --features jit,event-scheduler --lib jit_framework::
cargo test -p labwired-core --features event-scheduler --lib
```

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(jit): native loop-trace execution — cap at window and SysTick edges, charge k*n cycles"
```

---

### Task 6: Real-lab differential with the shipped RP2040 demo (native CLI, jit-core)

**Files:**
- Create: `crates/cli/tests/fixtures/rp2040_loop_demo/` (build.sh + .S + .elf, copied from the `pio_parked_chain` fixture pattern)
- Create: `crates/cli/tests/cortex_m_loop_trace_lab.rs`
- Modify: `crates/cli/Cargo.toml` (`required-features` on the new test target: `["jit-core"]`)

- [ ] **Step 1: Fixture**

`crates/cli/tests/fixtures/rp2040_loop_demo/build.sh` mirroring `pio_parked_chain/build.sh`; `loop_demo.S` implements the demo shape at scale: `movs r4,#0xFF`, loop of `bl nop; adds; subs; bne` 1,000,000 times is too long — use 10,000 outer × inner counted loop and toggle GPIO25 every outer pass (the RP2040 LED pin on `configs/systems/rp2040-pico.yaml`), ending in a `b .` park. Reuse the `esp32_ccount_blink` fixture's UART/GPIO report style if simpler.

- [ ] **Step 2: Test**

Model on `cortex_m_poll_loop_skip.rs`: run the fixture through the CLI with `LABWIRED_CORTEX_M_JIT=1` and (a) traces on, (b) traces off via a new `LABWIRED_CORTEX_M_JIT_TRACES=0` escape hatch read in `apply_run_speed_opts` (`crates/cli/src/commands/test.rs` — same pattern as `arm_jit`); compare result.json except `metrics.pc_samples`; assert the traces-on run is not slower and `[batched]`/stats show trace iterations (add a `trace_iters` suffix to the `labwired-cli test` summary line when non-zero, printed like `idle_ff=`).

- [ ] **Step 3: Run**

Run: `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 CARGO_TARGET_DIR=/Volumes/LabWired/.cargo-targets/core-lt cargo test -p labwired-cli --features jit-core --test cortex_m_loop_trace_lab`
Expected: PASS; record wall-time before/after in the commit message.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "test(cli): RP2040 loop-lab differential — traces report the same run, faster"
```

---

### Task 7: Browser runtime + the local perf number

**Files:**
- Modify: `crates/wasm/src/jit_browser.rs`

- [ ] **Step 1: Cap staging and read-back**

- `CortexMBrowserBlock::run(cpu, ram)` gains `iters_cap: Option<u32>`; write `bytes[ITERS_CAP_SLOT as usize..+4]` (LE) before `regs_view.copy_from(&bytes)` (the register bytes array already covers 96 bytes — the cap slot at 84 is inside it); after the call parse `bytes[ITERS_DONE_SLOT..+4]` into `iterations_done` and return it with the existing tuple.
- `BrowserJitCache` gains `trace_runs: u64, trace_iters: u64`; expose `WasmSimulator::jit_trace_iters()` next to `jit_hits()` in `crates/wasm/src/lib.rs`.
- `try_browser_cortex_m_jit_step`: mirror the native cap computation (budget / per-iter, `systick_ticks_until_fire`, `MAX_TRACE_ITERS`), call with the cap for traced plans, and return `done * per_iter` instead of `instr_count`; bump the trace stats.
- `WasmSimulator` gains `set_jit_loop_traces_enabled(bool)` (next to `set_jit_enabled`, same doc style) that flips `SimulationConfig::cortex_m_jit_loop_traces`; `WasmSimulator` must route the traced frontend entry exactly as Task 5 Step 3.

- [ ] **Step 2: Build + bench (the number this plan exists for)**

```bash
cd crates/wasm && CARGO_TARGET_DIR=/Volumes/LabWired/.cargo-targets/wasm-jit2 wasm-pack build --release --target web --out-dir /tmp/loop-trace-engine
```

Then, reusing `~/…/scratchpad/bench/rp2040-bench.mjs` (same env: `SYS` = lwbench rpi-pico system, `CHIP` = `configs/chips/rp2040.yaml`), call `sim.set_jit_enabled(true); sim.set_jit_loop_traces_enabled(true);` and measure `demo-rp2040-pico.elf` 2 M cycles:
Expected: ≥ 4x the interpreter baseline (8.4 M cycles/s on this Mac), i.e. 30 M+ cycles/s, with `jit_trace_iters()` > 0. Record both numbers in the commit message.

- [ ] **Step 3: Parity in the real browser worker**

Re-run the scratchpad `cdp-run.mjs` rig against a locally served playground build with traces on (or the labwired browser-layer gate in Task 8), and the existing `cargo test -p labwired-wasm --lib`.

- [ ] **Step 4: Commit + PR**

```bash
git add -A
git commit -m "feat(wasm): loop-trace execution in the browser Cortex-M JIT"
```

---

### Task 8: Labwired side — opt-in, gate, measure, then default

**Files (labwired repo, separate PRs):**
- `packages/wasm-runtime/src/*` — expose the new `set_jit_loop_traces_enabled` through the runtime's simulator wrapper.
- Playground lab runner — enable traces (and `set_jit_enabled(true)`) for Cortex-M labs behind a `?jit=1` / config flag first.
- `packages/playground` tests — a browser-layer case that asserts trace iterations > 0 and identical results for the RP2040 demo.

- [ ] **Step 1:** core bump PR in `w1ne/labwired` (same procedure as #2584: gitlink + `scripts/ci/regenerate-core-derived.sh run` + live-manifest check after deploy).
- [ ] **Step 2:** flip the flag for the RP2040 demo lab, run the browser-layer gate, measure `demo-rp2040-pico` real time in the deployed worker rig (`prod-worker` style), compare against the recorded 0.10–0.15x baseline.
- [ ] **Step 3:** only if the measured gain holds in prod and the differentials stay green for a week of main, flip the default for Cortex-M labs and note it in `hub_browser_runtime.md`.

---

## Self-review notes

- **Spec coverage:** cap staging (Tasks 3/5/7), accounting `k*n` (5/7), IRQ/budget clamps (5/7), inlined leaf calls (3), refusals for memory/VFP/IT (3), non-vacuous stats (1/5/7), negative control (5), real-lab evidence (6/8), no native behavior change when the flag is off (4/5 use the new entry only behind the flag).
- **Deliberate V1 limits:** no memory, no VFP, no IT, no `BlxReg`, one inlining level, self-target backedge only. Each refusal falls back to the existing basic-block path, byte-identical to today.
- **Open decision the executor must confirm before Task 5:** whether `run_jit_loop`'s existing `block_would_cross_irq(bus, n)` pre-check stays as-is for non-traced blocks (yes) and is replaced by the cap computation for traced blocks (yes) — read the current code and keep the non-traced branch byte-for-byte.
