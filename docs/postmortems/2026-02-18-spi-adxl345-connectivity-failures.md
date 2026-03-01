# Postmortem: SPI & ADXL345 Connectivity Failures

**Date**: 2026-02-18
**Status**: Resolved
**Authors**: @andrii, @antigravity

## Issue Summary
The initial attempt to simulate the ADXL345 sensor over SPI on the nRF52832 target failed. The firmware correctly initialized the peripheral and performed transfers, but the returned Data Register (`DR`) value was either `0x00` or inconsistent, leading to a failure of the mandatory `DEVID` check (Expected: `0xE5`).

## Impact
- **Severity**: Medium (Impacts serial protocol simulation reliability).
- **Release**: v0.13.0 will include the architectural fixes.
- **Components Affected**: `Spi` core, `SystemBus` loader, sensor register logic.

## Root Cause Analysis

### 1. Missing Downcasting (Wiring Failure)
- **The Issue**: The `SystemBus::from_config` logic uses dynamic downcasting to "attach" an external sensor to a bus controller.
- **Discovery**: Real-time tracing showed "Attached ADXL345" info log was missing because `p.dev.as_any_mut().downcast_mut::<Spi>()` returned `None`.
- **Root Cause**: The `Spi` struct lacked an implementation of `as_any_mut`, making it an "opaque" peripheral that the bus couldn't wire stuff into.

### 2. Double-Trigger Bug (Atomicity Violation)
- **The Issue**: A 16-bit store instruction to the SPI `DR` (offset `0x0C`) was being split into two 8-bit writes by the internal bus simulation.
- **Discovery**: Tracing showed two "Command Received" logs from the ADXL345 for a single firmware transfer.
- **Root Cause**: The `Spi::write` method triggered a `write_reg` side-effect on *every* byte write. For multi-byte registers, this caused the "start transfer" logic to execute twice, once with potentially incomplete data.

### 3. RXNE Clearing Synchronization
- **The Issue**: The `RXNE` (Receive Buffer Not Empty) flag was being cleared incorrectly or persisting across transactions.
- **Root Cause**: The master controller wasn't clearing `RXNE` on the read of `DR`. In a real register interface, reading the data register is the hardware signal that the software has consumed the data.

### 4. Integer Overflows (Rust Safety)
- **The Issue**: Simulation panicked during certain sub-word register access.
- **Root Cause**: Calculations like `val << (byte_offset * 8)` were performed on `u16` types. When `byte_offset` was `2` (offset `0x0E`), this shifted by 16 bits, which is out of range for `u16`.

## Resolution
- **Standardized Trait**: Implemented `as_any_mut` in `spi.rs` to enable system manifest connectivity.
- **Debounced Side-Effects**: Modified `Spi::write` to only trigger `write_reg` side-effects if the `offset` matches the **base address** of the register.
- **Standardized Mutable Read**: Propagated `&mut self` to `Peripheral::read` to allow hardware side-effects (clearing `RXNE`) during the data phase.
- **Promoted Arithmetic**: Forced all register bitwise operations to `u32` to ensure shift-safety.

## Permanent Mitigation
1.  **Macro-Driven Boilerplate**: Implement a `#[derive(Peripheral)]` macro to automatically generate `as_any`, `as_any_mut`, and `snapshot` boilerplate.
2.  **Transaction-Aware Bus**: Update `SystemBus` to provide a "Transactional Hint" to peripherals, indicating if a write is part of a wider store instruction.
3.  **Strict Integer Policy**: Audit all peripheral `read`/`write` implementations for potential shift overflows.
