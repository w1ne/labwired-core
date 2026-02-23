# Postmortem: Cortex-M ISA Coverage Gap in HIL Displacement Showcase

**Date**: 2026-02-23
**Status**: Resolved
**Related Artifacts**: [VALIDATION.md](file:///home/andrii/Projects/labwired/core/examples/hil-displacement-showcase/VALIDATION.md)

## Summary
During the implementation of the **HIL Displacement Showcase**, the LabWired simulation platform encountered several "Unknown Instruction" errors and simulation stalls. These failures were traced to missing Thumb-32 instructions and incorrect peripheral IRQ mappings in the `stm32h563.yaml` chip descriptor.

## Identified Gaps

### 1. Instruction Set Coverage
Several critical Thumb/Thumb-32 instructions were missing from the `CortexM` emulator:
- **`STRB` (Register Offset)**: Used by the HAL for buffer management.
- **`CMP.W` (Data Processing Immediate)**: Thumb-32 variant for large immediate comparisons.
- **`BKPT`**: Used for signaling simulation halt.

### 2. Peripheral IRQ Mapping
The `stm32h563.yaml` descriptor incorrectly mapped `DMA1` to IRQ 11. In Cortex-M, IRQs < 16 are reserved for internal exceptions. This mapping caused:
- A spurious **SVCall** (Exception 11) trigger instead of a DMA interrupt.
- A memory access violation when the CPU jumped to an uninitialized vector table offset.

### 3. DMA Routing
The `SystemBus` lacked routing for `uart3` DMA signals, which prevented the HIL stress test (a mem-to-periph transfer) from completing.

## Resolutions
- **ISA Expansion**: Implemented `STRBReg`, `BKPT`, and a comprehensive `DataProcImm32` handler that correctly updates NZCV flags.
- **Descriptor Hardening**: Corrected the `stm32h563.yaml` to remove illegal IRQ assignments.
- **Routing Correction**: Updated `bus/mod.rs` to support `uart3` TX DMA signals.

## Lessons Learned
- **High-Fidelity Demands Precision**: Moving from general "hello world" demos to HIL stress tests exposes subtle ISA completeness gaps.
- **Validator Gate Improvements**: The **Top-20 Coverage Matrix** should be strictly enforced before onboarding new high-performance chips.
- **Aether Debugger Utility**: Using the **Aether** debugger parity check was instrumental in confirming the IRQ mapping mismatch between real hardware and the initial model.

## Status
The HIL Displacement Showcase is now **PASSING** with cycle-accurate timing and correct DMA behavior.
