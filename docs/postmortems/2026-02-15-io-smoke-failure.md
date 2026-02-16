# Postmortem: STM32H563 GPIO ODR Failure (io-smoke)

**Date**: 2026-02-15
**Status**: Resolved
**Authors**: @w1ne, @antigravity

## Issue Summary
The `io-smoke` regression test for the `nucleo-h563zi` board failed to assert the expected LED states. The firmware reported `PB0=0` even when the simulator's internal state showed the ODR was correctly set to `1`. This led to an initial investigation into GPIO peripheral logic, but the actual root cause was found in the CPU simulation.

## Impact
- **Severity**: High (Initially thought to be Medium, but CPU instruction failures have broad impact).
- **Release**: v0.12.0 will proceed AS PLANNED with this fix included.
- **Tests Affected**: `examples/nucleo-h563zi/io-smoke.yaml`.

## Root Cause Analysis

### 1. GPIO Peripheral (Verified Correct)
- The ODR verification logs confirmed that the peripheral was correctly processing `BSRR` writes and updating the `ODR` value.
- The suspicion regarding `GpioPort` write buffering was a **false positive**.

### 2. CPU Simulation (Actual Root Cause)
- **Missing `IT` (If-Then) Support**: The Thumb-2 `IT` instruction was previously decoded as a `Nop`. In the guest firmware, conditional logic (e.g., `movpl r1, #48` inside an `IT PL` block) was being executed unconditionally or skipped incorrectly.
- **Incomplete Thumb-2 Decoding**: Instructions like `MOVW`, `MOVT`, and 32-bit `LDR`/`STR` variants were not correctly handled by the modular decoder, leading to inconsistent register state during the GPIO-to-UART reporting phase.
- **Result**: The firmware's reporting loop was correctly reading the `ODR` bit but incorrectly processing the character to send over UART because the conditional "if bit is 1, send '1'" logic was failing at the CPU level.

## Resolution
- **`IT` Block State Management**: Implemented `it_state` in the `CortexM` step loop to handle conditioned instructions correctly.
- **Architectural Decoder Expansion**: Updated `arm.rs` to handle `IT`, `MOVW`, `MOVT`, `LDR.W`, `STR.W`, and `UXTB.W` instructions.
- **Verification**: The `io-smoke` test now passes with `PB0=1`.

## Timeline of Attempts
1.  **Infinite Loop Fix**: Fixed delay loop with `black_box`.
2.  **Binary Freshness**: Forced rebuild with correct target.
3.  **RCC Enable**: Enabled clocks in firmware.
4.  **Diagnostic Trace**: Added register write logging; identified that `MOVPL` was executing when it shouldn't have, pointing to `IT` failure.
5.  **Fix**: Implemented `IT` state machine and Thumb-2 instructions.

## Permanent Mitigation
The following strategies have been integrated into the v0.2.0 roadmap to prevent future silent instruction-level regressions:
1.  **Automated ISA Audit**: CI-time disassembly check of guest binaries to verify all used opcodes are implemented.
2.  **Strict CPU Trap Mode**: Configurable `--strict-cpu` flag to Promote `Unknown` instructions to hard panics during tests.
3.  **Exhaustive Conformance Tests**: Integration of ARM Thumb-2 test suites for bit-accurate verification.
4.  **HIL Trace Comparison**: Automated diffing of simulation traces against real hardware (Nucleo-H563ZI) logic analyzer logs.
