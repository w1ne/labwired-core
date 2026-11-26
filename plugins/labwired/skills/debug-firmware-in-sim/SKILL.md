---
name: debug-firmware-in-sim
description: Reproduce and fix embedded firmware bugs deterministically in LabWired simulation using UART, GPIO, I2C/SPI/analog traces, stimuli, and interactive debugging.
---

# Debug Firmware in Simulation

Use this skill when embedded firmware compiles but misbehaves (hangs, prints wrong values, misses interrupts, fails I2C/SPI handshakes, toggles the wrong pin, or crashes under edge-case inputs). LabWired lets you reproduce the failure deterministically without physical hardware and prove the fix.

## Workflow

### 1. Capture Circuit Ground Truth
- Call `labwired_describe` (or `labwired_context` / `labwired_project_get`) to verify exact MCU pin assignments, bus wiring (I2C/SPI/UART/CAN), pull-ups, and peripheral addresses.
- Run `labwired_validate` first — many "firmware bugs" are actually swapped TX/RX, wrong I2C address, missing pull-up, or floating reset pins.

### 2. Reproduce with Observability (`labwired_run`)
- Compile the current firmware with `labwired_compile` (or attach an existing ELF with `labwired_put_firmware`).
- Call `labwired_run` with:
  - `watch_gpio` listing the pins involved (LEDs, CS, IRQ, PWM, reset)
  - `stimuli` to replay external triggers (button press/release, sensor voltage/value changes) at exact timestamps
  - `uart_injections` to feed host commands over serial
  - `acceptance` assertions capturing the expected behavior so the run stops cleanly when met or reports exact failure state
- Examine the returned evidence:
  - Full UART TX transcript
  - GPIO edge counts and transition timelines
  - Decoded I2C/SPI/UART/CAN bus traffic and interface advisories (NACKs, bus stuck low, clock mismatch, unaddressed slave)
  - Analog/SPICE probe readings when analog circuits or sensors are attached

### 3. Deep Inspection & Interactive Stepping
When a single run does not reveal the root cause:
- Call `labwired_inspect` to read peripheral states, memory, and pin states from the simulation snapshot.
- For crashes, hardfaults, or race conditions, start an interactive debug session:
  1. `labwired_debug_start`
  2. `labwired_debug` to set breakpoints, single-step, inspect CPU registers, stack pointer, and peripheral registers
  3. `labwired_debug_stop` when finished
- If a real hardware bench is attached and you need to cross-check silicon parity, use `labwired_device_list`, `labwired_device_flash`, `labwired_device_console`, `labwired_device_inspect`, and `labwired_device_set_source_breakpoints`.

### 4. Isolate Root Cause Before Patching
State the concrete root cause backed by observed evidence (e.g., "GPIO4 never toggled because `DDRD` bit 4 was cleared; I2C write to `0x3D` NACKed because the display strap is wired to `0x3C`"). Do not apply blind retries.

### 5. Patch and Lock In a Regression Check
- Apply the minimal firmware fix via `labwired_project_edit_firmware` or `labwired_put_source`.
- Re-compile with `labwired_compile`.
- Run `labwired_verify` with an oracle assertion that would have **failed before the fix** and **passes after the fix**.
- Report the exact root cause, the diff applied, and the verified before/after evidence.
