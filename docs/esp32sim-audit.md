# Technical Audit: Selective Adoption from `mikroverk/esp32sim`

This document records the engineering audit of [`mikroverk/esp32sim`](https://github.com/mikroverk/esp32sim) (`v0.4.0` through PR `#383` / EX270, MIT License) against `labwired-core`, what we adapted, what we verified as already covered or out of scope, and the reproducible benchmark numbers.

## Attribution & License

Adapted concepts and decoded basic-block cache design credit:

- **`mikroverk/esp32sim`** — MIT License  
  Copyright (c) 2026 Joakim Eriksson (`@joakimeriksson`), Alice (`@aliceisjustplaying`) and Mattia Fiumara (`@mfiumara`)
- `crates/core/src/cpu/riscv_block_cache.rs` adapts the direct-mapped decoded basic-block cache layout from `esp32sim`'s `riscv-rv32/src/block.rs` (EX265) and extends it with LabWired's compressed RVC/QingKe decoding, per-store IRAM invalidation, and CLINT interrupt-boundary checks.
- `crates/core/src/cpu/jit_framework/riscv/exec.rs` adapts the persistent shared Wasmtime `Store` + `Memory` execution model inspired by `esp32sim`'s `riscv-rv32/src/jit.rs` onto LabWired's existing `jit_framework/riscv` compiler.

## Adopted Improvements

### 1. Portable RV32 Decoded Basic-Block Cache (`Rv32BlockCache`)

- **Source:** `crates/core/src/cpu/riscv_block_cache.rs`, integrated in `crates/core/src/cpu/riscv.rs`.
- **Design:** 2,048-entry direct-mapped block table backed by a flat 32 Ki-instruction arena (blocks up to 32 instructions, generation-counted reset, `FLAG_HAS_MEM` / `FLAG_HAS_STORE` / `FLAG_SELF_LOOP` annotations). Runs pure Rust with zero JIT dependencies, so it accelerates both native CLI/CI and browser WASM32 execution equally.
- **Self-modifying code & bus safety (inspired by `esp32sim` PR `#383` / EX270):** Instead of adding a per-SoC bus trait, `RiscV::invalidate_fetch_if_store_overlaps` calls `block_cache.invalidate_if_store_overlaps(addr, size)` on every store to RAM/IRAM, and `invalidate_fetch_window` drops cached blocks globally on flash/code mutations. Every block exit checks `block_would_cross_irq` (MIE + CLINT `mtime`/`mtimecmp`) and `event-scheduler` deadlines so interrupt delivery remains cycle-exact.
- **Verification:** Differential register + memory equivalence against the baseline interpreter and an explicit self-modifying IRAM regression test (`block_cache_invalidates_when_guest_modifies_iram_code`) in `crates/core/tests/riscv_block_cache_and_shared_jit_bench.rs`.

### 2. Shared Persistent Wasmtime `Store` + `Memory` in `RiscvJitEngine`

- **Source:** `crates/core/src/cpu/jit_framework/riscv/exec.rs`.
- **Problem solved:** LabWired's previous `RiscvWasmJit` instantiated a separate `wasmtime::Store` + `Memory` per compiled block and copied the entire guest RAM slice in and out on every block call. On ESP32-C3/C6/P4 targets (400–768 KiB SRAM), that per-block memcpy dominated execution time.
- **Change:** All compiled blocks in `RiscvJitEngine` now share one persistent `Store` and one growable linear `Memory` (`SharedCompiledBlock`). Guest RAM is synced into the shared linear memory once at JIT streak entry and flushed back once (`flush_jit_ram`) only when returning to the interpreter or hitting MMIO/uncompiled paths.

## Reproducible Benchmarks

Run on Linux x86_64 (`cargo test --release --features "jit-framework,jit"`):

```sh
cargo test -p labwired-core --release --features "jit-framework,jit" \
  --test riscv_block_cache_and_shared_jit_bench -- --include-ignored --nocapture

cargo test -p labwired-core --release --features "jit-framework,jit" \
  --test riscv_jit_combined_bench -- --ignored --nocapture
```

| Workload | Baseline interpreter (`decode_cache=false`) | `Rv32BlockCache` (native & browser WASM32) | `RiscvWasmJit` (shared `Store`+`Memory`) |
| --- | ---: | ---: | ---: |
| ESP32 512 KiB SRAM ALU + mem + branch (2M instrs) | 35.8 Minsn/s (55.90 ms) | **193.8 Minsn/s** (10.32 ms, **5.42×**) | **85.8 Minsn/s** (23.30 ms, **2.40×**) |
| RV32IM pure ALU self-loop (2M instrs) | 69.8 Minsn/s (28.64 ms) | **402.9 Minsn/s** (4.96 ms, **5.77×**) | 59.8 Minsn/s (33.47 ms, 0.86×*) |
| Short 256 B SRAM `step_unit` vs `Machine::step` (1M instrs) | 5.32 Minsn/s (187.8 ms) | — | **153.6 Minsn/s** (6.51 ms, **28.85×**) |

\* On a 6-instruction tight self-loop, crossing the host→Wasmtime FFI boundary per block costs more than chaining the loop directly in Rust inside `Rv32BlockCache`. `Rv32BlockCache` is therefore the default fast path for both native and browser WASM32 builds, while `RiscvWasmJit` remains available for larger linear compute blocks when `--features jit-framework,jit` is enabled.

## Audited & Intentionally Skipped

| `esp32sim` component | Audit verdict | Reason & existing `labwired-core` coverage |
| --- | --- | --- |
| **ESP32-P4 + lock-step C6 WiFi (`esp32p4/src/coproc.rs`)** | Skipped (already modeled differently) | `labwired-core` already defines dual-core HP/LP ESP32-P4 (`configs/chips/esp32p4.yaml`, `examples/esp32p4x-c5-function-ev-board/`) and WiFi MAC host bridging (`docs/esp32c3_wifi_mac_bridge.md`) on top of our multi-machine bus architecture. Upstream's `coproc.rs` is tightly coupled to a single-SoC crate layout. |
| **ESP32-C3 / C6 mask-ROM boot** | Skipped (redundant) | `labwired-core` already implements mask-ROM boot and ROM function dispatch in `crates/core/src/boot/esp32c3_rom.rs` and `crates/core/src/boot/esp32c6_rom.rs` (201 ESP32-C3 and 54 ESP32-C6 unit tests passing). |
| **ESP32-S31 (`esp32s31/src/rom.rs`)** | Skipped (insufficient fidelity) | Upstream's ESP32-S31 support is an early 184-line ROM stub for pre-production silicon. Per `FIDELITY.md`, we defer onboarding new chips until public vendor TRM/SVD and runnable silicon-checked firmware exist. |
| **Upstream Wasm bytecode emitter (`riscv-rv32/src/jit.rs`)** | Skipped (redundant) | `labwired-core` already ships a full RV32→Wasm emitter (`crates/core/src/cpu/jit_framework/riscv/emit.rs` + `host.rs` + `MIN_PROFITABLE_BLOCK_INSTRS`) and word-cached MMIO (`crates/core/src/bus/mmio_words.rs`). Only the shared persistent `Store`+`Memory` execution model was adopted. |
