# Troubleshooting Guide

This guide addresses common issues encountered when using LabWired Core.

## Simulation Issues

### "Memory Violation at 0x..."
**Symptom**: The simulator halts with `StopReason::MemoryViolation(address)`.
**Cause**: The firmware attempted to access an address that is not mapped to Flash, RAM, or any registered peripheral.
**Solution**:
1. Check your `chips/<chip>.yaml` definition. Does the address fall within the declared Flash or RAM regions?
2. Check your `system.yaml`. Is there a peripheral missing that should be at that address?
3. Verify the firmware linker script (`memory.x`). It might be placing data in valid hardware regions that the simulator doesn't know about yet.

### "Instruction Decode Error"
**Symptom**: `StopReason::DecodeError(address)`.
**Cause**: The CPU attempted to execute an instruction that is invalid or undefined for the current architecture (e.g., executing data as code).
**Solution**:
1. Check the **Vector Table**. Is the Reset Vector pointing to the correct entry point (usually `_start` or `Reset_Handler` + 1 for Thumb mode)?
2. Verify `load_firmware` succeeded. Run with `--trace` to see the first few instructions. If it crashes immediately, the entry point might be wrong.
3. Cortex-M requires the PC LSB to be 1 (Thumb mode). If your vector table has an even address (e.g., `0x08000100`), the CPU will switch to ARM mode and fault.

### "DAP Server Unreachable"
**Symptom**: VS Code displays "Connection refused" or "Timeout" when starting a debug session.
**Cause**: The LabWired instance isn't running, or the port is blocked.
**Solution**:
1. Ensure LabWired is started with `--gdb` or is in interactive mode.
2. Check the port (default `3333`). Is it in use by another OpenOCD/J-Link instance?
3. In `launch.json`, verify `miDebuggerServerAddress` matches `localhost:<port>`.

### "UART Output is Garbage"
**Symptom**: `uart.log` contains random characters.
**Cause**: Baud rate mismatch between firmware and simulator config.
**Solution**:
1. LabWired's UART model is currently "byte-perfect" (it doesn't simulate physical baud timing errors), but ensure your firmware is actually writing valid ASCII.
2. Check if `echo_stdout` is enabled or if you are looking at raw binary data.

## CI/CD Issues

### "Max Steps Reached"
**Symptom**: CI fails with `StopReason::MaxSteps`.
**Cause**: The proprietary `max_steps` limit in your test script is too low for the initialization code to complete.
**Solution**:
1. Increase `max_steps` in your `.yaml` test script.
2. Optimize your firmware boot sequence (e.g., reduce PLL validation timeouts).

### "No Progress / Stuck"
**Symptom**: `StopReason::NoProgress`.
**Cause**: The PC has stayed in the same range (or exactly the same address) for `no_progress_steps`. This usually means an infinite loop (e.g., `while(1) {}` or a HardFault handler).
**Solution**:
1. Use `--trace` to identify the loop.
2. If it's a valid strict polling loop, insert a `asm!("nop")` or similar to vary the PC, or increase `no_progress_steps`.
