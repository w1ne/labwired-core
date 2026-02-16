# Postmortem: v0.12.0 Release Regressions & CI Bypass

**Date**: 2026-02-16
**Status**: Investigating / Resolving
**Authors**: @antigravity

## Issue Summary
Immediately following the release of `v0.12.0`, several critical regressions were discovered:
1.  **Workspace Compilation Failures**: `labwired-loader`, `labwired-cli`, and `labwired-dap` contained regressions (duplicate methods, syntax errors, and missing type definitions) that prevented workspace-wide builds.
2.  **Instruction Set Regressions**: Missing 16-bit register shift variants (`LSL`, `LSR`, `ROR`) led to incorrect instruction mapping in the modular decoder, causing functional failures in `stm32h563` smoke tests.
3.  **Smoke Test Timeouts**: The `stm32h563` `io-smoke` test required significantly more cycles (up to 1M steps) than allocated (256 steps), causing early exits and false negatives.

## Impact
- **Severity**: Critical. The v0.12.0 release was technically unbuildable from source in a clean environment.
- **Affected Components**: `labwired-core`, `labwired-loader`, `labwired-cli`, `labwired-dap`, `stm32h563` example.
- **Remediation**: Releasing `v0.12.1` patch immediately.

## Root Cause Analysis

### 1. Loader, CLI, & DAP Instability
- **Duplicate Code**: During the refactoring of `SymbolProvider` in `labwired-loader`, duplicate `resolve_symbol` and `find_locals` definitions were left in the file.
- **Syntax Errors**: A late-stage manual edit to `labwired-cli` introduced an unclosed delimiter in the `build_bus` function.
- **Missing Definitions**: `labwired-dap` was missing the `BreakpointResolution` struct definition, likely deleted during a cleanup pass.
- **CI Gap**: CI only correctly validated `labwired-core` sub-crate in isolation or the user pushed without waiting for full workspace validation results.

### 2. ISA Mapping Regression (h563 Failure)
- **Introduction**: This regression was introduced in commit **`754398b`** ("emergency sync develop with v0.12.0") on Feb 16, 2026.
- **Shadowing**: The Thumb-2 `ShiftReg32` variant was added and incorrectly used to map Thumb-1 16-bit register shifts. This caused a +4 PC increment instead of +2, leading to immediate instruction stream corruption.
- **Missing Thumb-1 Variants**: Commit `754398b` also missed the entire family of Thumb-1 Load/Store Register Offset instructions (`STR`, `LDR`, `STRB`, `LDRB`, etc. with register offsets), which are essential for many standard C runtime loops (causing `fullchip-smoke` to fail).

### 3. CI Bypass Mechanics
- **Integration Test Loophole**: `strict_onboarding.rs` skips firmware builds for examples lacking a `Cargo.toml`. Since `nucleo-h563zi` uses a `Makefile`, the smoke test might have been running against stale binaries or skipped entirely if the binary path wasn't found.
- **Target Mismatch**: `core-ci.yml` adds `thumbv7m-none-eabi` but doesn't explicitly add targets for all onboarded chips, leading to silent build failures in integration tests.

### 4. Second Pass Regressions (Config, DAP, & Demos)
- **YAML Literal Regressions**: Commit `754398b` introduced a stricter YAML parser (or configuration) that rejects underscores in numeric literals (e.g., `0x4000_C000`) and the `K`/`M` aliases in size fields (requiring `KB`/`MB`). This broke `riscv-virt` and `arm-cortex-m0` configurations.
- **DAP Version Drift**: `labwired-dap` was left at `v0.11.0` while the rest of the workspace moved to `v0.12.0`, leading to struct initialization errors (`schema_version` mandatory field missing).
- **Demo Blinky Stall**: Functional tests for `demo-blinky` failed because the 500,000-iteration delay loop (~2.5M instructions) exceeded the default 500,000 step limit in the integration test, leading to false-negative functional failures.

## Resolution
- **Fixes**:
  - Traceability: Deduplicated loader methods and fixed CLI syntax.
  - ISA: Implemented missing Thumb-1 Register Load/Store family in `arm.rs` and `cortex_m.rs`.
  - Config: Restored `riscv-virt` and `arm-cortex-m0` by normalizing numeric and size literals.
  - DAP: Synchronized `labwired-dap` with workspace version `v0.12.1` and fixed test initializers.
  - Demos: Increased `demo-blinky` test limits to 5,000,000 steps to account for realistic delay loops.
- **Verification**:
  - Verified `nucleo-h563zi` `fullchip-smoke` PASS.
  - Verified `arm-c-hello` (C-based pointer logic) PASS.
  - Refactored `strict_onboarding.rs` to close the Makefile loophole.
- **Patch**: Version bumped to `v0.12.1`.

## Permanent Mitigations & Release Hardening
1.  **Strict Workspace CI**: Update `core-ci.yml` to run `cargo check --workspace` and `cargo test --workspace`.
2.  **YAML Schema Enforcement**: Add a CI step that validates all `*.yaml` files against the `labwired-config` parser.
3.  **VS Code Extension CI**: Integrate vscode extension checks into the main CI pipeline. The extension MUST be tested against the latest DAP build to ensure protocol compatibility.
4.  **Updated Release Instructions**:
    *   **Pre-Tag Checklist**:
        *   [ ] Full workspace build: `cargo build --workspace`
        *   [ ] Exhaustive test: `cargo test --workspace` (including DAP and loader crates)
        *   [ ] Example Audit: Run `./core/scripts/run_all_smoke_tests.sh` to verify all hardware examples.
        *   [ ] Configuration check: Run `labwired-cli validate-configs` (to be implemented).
    *   **Extension Bump**: Every DAP protocol change MUST be accompanied by a version bump in the VS Code extension `package.json`.
    *   **Post-Release Audit**: Run headful VS Code extension smoke tests in the local environment before announcing the release.
