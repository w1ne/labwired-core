# Thumb JIT frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development.

**Goal:** All-bail Thumb `IsaFrontend` + lockstep vs Cortex-M interpreter, cloned from the RISC-V foundation.

**Architecture:** New `crates/core/src/cpu/jit_framework/thumb/` mirroring `riscv/`. Decode via `crate::decoder::arm`. Tests under `jit-framework`.

**Work from:** `/tmp/labwired-thumb-jit` branch `feat/thumb-jit-frontend`.

**Templates (copy structure, do not copy RV encodings):**
- `crates/core/src/cpu/jit_framework/riscv/mod.rs`
- `crates/core/src/cpu/jit_framework/riscv/host.rs`
- `crates/core/tests/riscv_jit_lockstep.rs`
- `crates/core/src/cpu/jit_framework/mod.rs` (add `pub mod thumb`)
- `crates/core/Cargo.toml` `[[test]]` `riscv_jit_lockstep`

---

### Task 1: All-bail Thumb walker

**Files:**
- Create: `crates/core/src/cpu/jit_framework/thumb/mod.rs`
- Create: `crates/core/src/cpu/jit_framework/thumb/host.rs` (can be stub snapshot if Task 2 owns host — prefer both here if small)
- Modify: `crates/core/src/cpu/jit_framework/mod.rs` — `pub mod thumb;`
- Create: unit tests in `thumb/mod.rs` `#[cfg(test)]` or `crates/core/tests/thumb_jit_walk.rs` with `required-features = ["jit-framework"]`

- [ ] **Step 1: Failing test** — assemble Thumb-16:
  - `MOV r0, #1` (0x2001)
  - `ADDS r0, r0, #1` (0x1C40) — verify encoding against `decode_thumb_16`
  - `B .-4` or similar loop

  Assert `ThumbFrontend.translate_block(0, view)`:
  - `entry_pc == 0`
  - `instr_count >= 2`
  - `plan.is_stub()` / empty `code`
  - last exit is ControlFlow or PartialBlock (branch)

  Assert classify: ADD sequential, B control-flow, WFI (0xBF30) unmodeled (block cut before it).

- [ ] **Step 2: Run with `--features jit-framework`, expect FAIL** (no `thumb` module).

- [ ] **Step 3: Implement walker** using `decode_thumb_16` / `decode_thumb_32` (32-bit when high halfword is a Thumb-2 prefix, same rule the DAP uses). `MAX_BLOCK_INSTRS = 1024`. Empty wasm body. `isa_name()` = `"thumb2"`.

- [ ] **Step 4: Tests PASS.** `cargo test -p labwired-core --features jit-framework --test thumb_jit_walk` (or module tests).

- [ ] **Step 5: Commit** `feat(jit): all-bail Thumb frontend walker`

---

### Task 2: CortexM host + lockstep gate

**Files:**
- Modify: `thumb/host.rs` — snapshot R0–R12, SP, LR, PC, xPSR
- Create: `crates/core/tests/thumb_jit_lockstep.rs` with `required-features = ["jit-framework"]`
- Modify: `crates/core/Cargo.toml` add `[[test]] name = "thumb_jit_lockstep"`

Mirror `riscv_jit_lockstep.rs`: two `Machine<CortexM>` with the same tiny loop in flash, `DispatchLoop` + `InterpreterRuntime` + `ThumbFrontend` vs pure `step` interpreter, compare `snapshot_state` each instruction for N steps (e.g. 64).

Need a valid Cortex-M reset: vector table at 0 (SP, reset PC) then loop at reset. Look at existing Cortex-M unit tests for the smallest machine setup.

- [ ] TDD: write lockstep test, fail, implement host + dispatch glue, pass.
- [ ] Commit `test(jit): Thumb all-bail lockstep vs interpreter`

---

No wasm. No Machine production wiring. No tlib.
