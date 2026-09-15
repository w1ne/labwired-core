# Cortex-M Thumb JIT frontend (foundation)

**Date:** 2026-09-15  
**Status:** implement (user: do the ARM JIT recommendation with subagents)

## Goal

Do **not** take tlib. Add a Thumb-2 [`IsaFrontend`] in `cpu/jit_framework`, matching the RISC-V all-bail foundation: walk + classify + empty wasm, lockstep vs the Cortex-M interpreter.

## Why

`jit_framework/frontend.rs` already lists Thumb-2 first. RISC-V landed; Thumb did not. That is why a hot ALU loop is slower than Renode tlib. Interpreter stays the spec.

## Milestone 1 (this change)

All-bail frontend, like `jit_framework/riscv`:

- Reuse `decoder::arm::{decode_thumb_16, decode_thumb_32}`.
- Walk a basic block over a `CodeView`, classify Sequential / ControlFlow / Unmodeled.
- Sequential: ALU, MOV, CMP, LDR/STR that we will later emit.
- ControlFlow: B, Bcc, BL, BX, BLX — end the block including that insn.
- Unmodeled: WFI, SVC, MRS/MSR, CPS, IT, unknown — cut **before**.
- `BlockPlan.code` empty; every block side-exits to the interpreter.
- `CortexMJitHost` + `snapshot_state` for R0–R15, xPSR, PRIMASK (same idea as `riscv/host.rs`).
- Lockstep test (`jit-framework` feature): tiny Thumb loop (adds + branch back) interpreter vs dispatch loop, state identical each retired insn.

## Non-goals this milestone

- No wasm emission / wasmtime.
- No Cortex-M dispatch wired into `Machine::advance` production path.
- No tlib.

## Later (not this PR)

Emit ALU then loads then branches; re-run the same lockstep gate; then wire `Machine<CortexM>` under `jit`.
