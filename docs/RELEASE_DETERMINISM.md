# Release-Gated Determinism Report

## 1. Introduction: The Determinism Mandate

In the era of Agentic AI, fuzzing, and complex Cyber-Physical Systems (CPS), simulation is only valuable if it is strictly reproducible. If a Reinforcement Learning agent discovers a zero-day vulnerability at epoch 40,000, that execution path must be 100% reproducible on a developer's local machine to be actionable.

LabWired guarantees **Strict Determinism**. A simulation run with the same Configuration, System Manifest, and Firmware Binary will produce the exact same observable state (registers, memory, output traces, and step count) across any supported host platform (Linux, macOS, Windows) and independent of host load.

This document outlines the architectural guarantees and the mandatory "Golden Board" release gating process that ensures this contract is never broken.

---

## 2. Core Architectural Guarantees

LabWired achieves determinism by decoupling simulation progress from the host machine's wall-clock time.

### 2.1 Instruction-Level Time Advancement
The fundamental unit of time in LabWired is the **Retired Instruction** (or simulated core clock cycle).
*   Peripherals do not use `std::time::Instant` or any host OS timers.
*   The `SystemBus` ticks peripherals strictly advancing based on the CPU's instruction execution budget.
*   If a host machine suffers a 500ms latency spike due to an OS context switch, the simulated CPU simply halts until the host recovers. The simulation time does not "drift."

### 2.2 The Lock-Free Actor Model and GVT
LabWired utilizes a message-passing Actor Model for multi-core and multi-peripheral simulation. To prevent race conditions between simulated actors running on different host threads, we enforce a **Global Virtual Time (GVT)** algorithm.
*   Actors compute local state speculatively.
*   State mutations (e.g., asserting an IRQ, writing to shared memory) are only committed when the GVT catches up to the actor's local time.
*   This ensures that concurrent events are resolved in the exact same order on an M2 Mac as they are on an x86 Linux CI runner.

### 2.3 Floating-Point Determinism
To avoid cross-platform floating-point discrepancies (e.g., fused multiply-add variations or 80-bit x87 precision leaks on older x86), LabWired strictly uses `softfloat` implementations for FPU instructions (like the Cortex-M4F `vadd.f32`), guaranteeing bit-exact IEEE-754 compliance across all targets.

---

## 3. The "Golden Board" Methodology

To systematically prevent regressions, the LabWired core team maintains a suite of **Golden Boards**. A Golden Board is a paired artifact consisting of:
1.  A pre-compiled, frozen firmware binary (e.g., `stm32f4_freertos_blinky.elf`).
2.  A frozen LabWired configuration (`test_script.yaml`, `system.yaml`).
3.  A **known-good signature** of the final execution state.

### 3.1 Trace Hashing (The State Signature)
The ultimate test of determinism is the Value Change Dump (VCD) trace. Because the VCD captures the state of every observable pin and register at every simulated cycle, it represents the complete sum of the simulation.

During the release process, the CI pipeline runs the Golden Boards and generates a `trace.vcd` and `result.json`. These files are then hashed (SHA-256).

If a pull request introduces an optimization that subtly changes the timing of a single instruction by one cycle, the resulting VCD hash will differ from the Golden Hash, and the CI build will fail immediately.

---

## 4. The Release Gate Checklist

Before any tag (e.g., `v1.2.0`) is cut and published to crates.io or GitHub Releases, the following mandatory steps are executed automatically via GitHub Actions:

- [ ] **Cross-Platform Verification**: The Golden Board suite is executed on `ubuntu-latest`, `macos-latest`, and `windows-latest`.
- [ ] **Hash Verification**: The SHA-256 hashes of the resulting `trace.vcd` and `result.json` artifacts for *every* Golden Board must match the version-controlled `expected_hashes.json` exactly across all three platforms.
- [ ] **Instruction Parity Check**: The `steps_executed` and `cycles` fields in `result.json` must exactly match the expected values.
- [ ] **RTL Co-simulation Sync**: (If applicable) Any Verilator-backed peripherals must complete their integration tests proving zero-copy IPC did not drop or reorder bus transactions.

### 4.1 Handling Legitimate State Changes
If a PR genuinely fixes a timing bug in a peripheral (meaning the previous "Golden" hash was technically incorrect behavior), the PR author must:
1.  Document the exact hardware justification for the timing change (referencing the vendor datasheet).
2.  Update `expected_hashes.json` in the same commit.
3.  The PR must be tagged as a `Behavioral Change` and cannot be merged as a generic patch without explicit maintainer approval.
