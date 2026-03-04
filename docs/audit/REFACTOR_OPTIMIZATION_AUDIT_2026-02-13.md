[← Back to Hub](../README.md)

# Refactor and Optimization Audit (2026-02-13)

This document tracks the focused runtime audit of `core/` and concrete fixes applied in this pass.

## Goals

1. Remove correctness hazards in debugger/runtime paths.
2. Improve runtime efficiency in hot paths.
3. Reduce panic-prone request handling in DAP server.
4. Keep behavior deterministic across different working directories.

## Findings and Actions

### 1. DAP profiling depth bug (critical)

- Issue: profiling call-tree depth derived from absolute SP value, which can explode path length and memory usage.
- Affected path: `core/crates/dap/src/server.rs`
- Fix:
  - Track an initial stack baseline and compute relative depth from that baseline.
  - Clamp depth growth to avoid pathological allocations.

### 2. DAP request argument handling (high)

- Issue: multiple `arguments.unwrap()` and nested unwraps could panic on malformed requests.
- Affected path: `core/crates/dap/src/server.rs`
- Fix:
  - Added validated argument access helpers.
  - Return protocol errors instead of panicking when required arguments are missing/invalid.

### 3. Unknown architecture fallback to ARM (high)

- Issue: unsupported/unknown ELF architecture silently defaulted to ARM.
- Affected path: `core/crates/dap/src/adapter.rs`
- Fix:
  - Fail fast with an explicit error for unknown architecture.

### 4. UART polling allocation churn (medium)

- Issue: UART poll cloned the buffer on every poll.
- Affected path: `core/crates/dap/src/adapter.rs`
- Fix:
  - Switched to `std::mem::take` to move bytes out without extra clone.

### 5. Peripheral lookup hot path (medium/high perf)

- Issue: per-byte memory read/write scanned all peripherals linearly.
- Affected path: `core/crates/core/src/bus/mod.rs`
- Fix:
  - Added a sorted peripheral range index plus a locality hint cache.
  - Read/write/peek paths now use indexed lookup instead of repeated full scans.

### 6. Descriptor path resolution determinism (medium)

- Issue: descriptor resolution checked current working directory before chip-relative resolution.
- Affected path: `core/crates/core/src/bus/mod.rs`
- Fix:
  - Prefer chip-relative path resolution first for relative descriptors.
  - Keep fallback to original relative path if chip-relative candidate does not exist.

### 7. DAP request parsing duplication (medium)

- Issue: command handlers repeated ad-hoc JSON extraction logic.
- Affected path: `core/crates/dap/src/server.rs`
- Fix:
  - Added typed request argument structs with shared JSON parse helper.
  - Reused common address parsing helper across memory/disassembly/goto handlers.

## Validation Commands

Run from `core/`:

```bash
cargo test -p labwired-core
cargo test -p labwired-dap
cargo clippy -p labwired-core -p labwired-dap --all-targets
```

## Remaining Follow-ups

1. Consolidate duplicated firmware `build.rs` boilerplate into a shared helper.
2. Add benchmarks for bus lookup and DAP command parsing to quantify gains.
