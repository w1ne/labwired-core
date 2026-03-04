[← Back to Hub](../../README.md)

# Postmortem: Prefix32 Performance Regression

**Date**: 2026-02-16
**Status**: Resolved
**Authors**: @antigravity

## Incident Summary
During the fix for STM32H563 GPIO regressions, 32-bit instructions (Thumb-2) were initially handled by decoding the first 16-bit halfword, returning a `Prefix32` placeholder, and then fetching the second halfword in a *subsequent* simulation step. While functionally correct, this caused a massive performance regression (approx. 2x) in instruction-heavy loops because every 32-bit instruction required two full `step()` calls.

## Impact
- **Performance**: 50% reduction in simulation speed for binaries using Thumb-2 extensions (standard for Cortex-M4/M7/M33).
- **Validation**: Smoke tests like `firmware-stm32f103-blinky` began timing out in CI because they exceeded the step limit, even though the guest code hadn't changed.
- **User Experience**: The CLI felt sluggish when running any complex firmware.

## Timeline
- **2026-02-15**: `Prefix32` introduced as a quick way to support modular decoding of 32-bit instructions.
- **2026-02-16 15:23**: User reports `firmware-stm32f103-blinky` is failing/timing out.
- **2026-02-16 16:10**: Analysis reveals that `Prefix32` is consuming a full cycles/step without executing the payload.
- **2026-02-16 17:45**: Refactored `CortexM::step` to handle the immediate fetch of the second halfword and execution of the 32-bit instruction in a single cycle.

## Root Cause
The simulator's `step()` function was designed around a 16-bit fetch model. To support 32-bit instructions without an instruction buffer, the decoder was allowed to return a "wait for next" signal (`Prefix32`). This ignored the fact that the simulation "cycle" is mapped to a `step()` call, effectively doubling the cost of 32-bit instructions.

## Corrective Actions
- **Fused Fetch/Execute**: Rewrote the 32-bit decoding path in `cortex_m.rs` to performs a look-ahead fetch if the first halfword indicates a 32-bit instruction.
- **Modular Decoder**: Consolidated all 32-bit logic into `decode_thumb_32` to ensure consistent handling across all crates.

## Prevention Gates
1. **Performance Benchmarks**: Added a requirement to track "Instructions Per Second" (IPS) in CI for standard demos.
2. **Step Limit Audit**: Regression tests must now specify *both* a time limit and a step limit; unexpected increases in step counts for the same binary will trigger a CI failure.
3. **ISA Awareness**: The modular decoder now explicitly warns if a 32-bit instruction is attempted in a 16-bit-only context (Cortex-M0).
