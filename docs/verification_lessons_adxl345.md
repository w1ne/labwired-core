# Case Study: High-Fidelity ADXL345 Verification

This document captures the technical lessons learned and infrastructure improvements implemented during the "Solid Proof" verification of the AI-generated ADXL345 peripheral asset.

## 1. Context
The goal was to achieve 100% formal verification of an AI-generated peripheral (ADXL345) including its register map, reset values, and synthesized side-effects (e.g., periodic interrupts).

## 2. Technical Hurdles and Solutions

### 2.1 Cycle-Accurate Batch Ticking
**Problem**: The LabWired test runner used high-speed batch execution for performance. However, peripherals were only being "ticked" once at the end of a batch, regardless of how many cycles the batch represented.
**Solution**: Re-implemented the execution loop in the LabWired CLI (`crates/cli/src/main.rs`) to track cycle-to-tick ratios. Peripherals are now ticked for every cycle elapsed, ensuring that timing-sensitive behaviors (like a 10-count heartbeat) fire accurately even in batch mode.

### 2.2 Multisize Memory Assertions
**Problem**: The `MemoryValue` assertion in the Simulation Protocol was implicitly 32-bit. This caused failures when verifying 8-bit registers at unaligned addresses, as the simulator would attempt a 32-bit read and return 0 (RAZ) for unmapped higher bytes.
**Solution**: Modified the protocol and runner to support explicit `size` parameters (8, 16, or 32 bits).
- **Update**: `MemoryValueAssertion` now includes a `size` field.
- **Harness**: The verification harness (`verify_harness.py`) now dictates the expected access width based on the register descriptor.

### 2.3 Intelligent Masking for Side-Effects
**Problem**: Periodic side-effects (heartbeats) can toggle status bits immediately after reset. Standard reset-value assertions failed because they expected a "clean" reset state (e.g., `0x00`), while the simulator correctly showed the bit already set (e.g., `0x80`).
**Solution**: Implemented "Conflict Masking" in the test script generator. The tool now calculates which bits are affected by `TimingAction` hooks and automatically masks those bits out during reset-value checks.

## 3. Simulation Fidelity Enhancements
To support "Solid Proofs", the following features were promoted to the stable `labwired` CLI:
- **`--vcd <PATH>` in `test` command**: Allows capturing cycle-accurate waveforms during headless verification.
- **Standardized Memory Mapping**: All verification projects now use the STM32-standard Flash (`0x08000000`) and RAM (`0x20000000`) locations to ensure compatibility with boilerplate driver firmwares.

## 4. Conclusion
By integrating these lessons, LabWired now supports a "Closed-Loop" AI ingestion pipeline where assets are not only generated but formally proven correct against high-fidelity simulation artifacts.
