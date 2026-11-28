---
name: build-and-verify-hardware
description: Build and deterministically verify a complete hardware system (circuit, firmware, BOM pricing, PCB, and 3D enclosure) using LabWired MCP.
---

# Build and Verify Hardware

Use this skill when the user wants to go from a hardware prompt or specification to a verified, working electronic system with real firmware, electrical validation, supplier BOM pricing, KiCad PCB layout, and an optional 3D enclosure — all via LabWired's generic hosted MCP tools at `https://api.labwired.com/mcp`.

## Prerequisites

- LabWired MCP server connected at `https://api.labwired.com/mcp` (see `https://app.labwired.com/tools/mcp` for 1-click setup).
- Free tier includes 10 verified builds/day and unlimited read/validate/check calls.

## Workflow

### 1. Discover Supported Targets and Components
Never invent part IDs or pin names. Query LabWired's live catalog first:
- Call `labwired_list` to discover available MCU boards, starter diagrams, and supported peripherals.
- Call `labwired_part_search` (and `labwired_part` / `labwired_datasheet`) when selecting sensors, displays, drivers, or passives so every pin name, supply rail, and electrical limit comes from real part metadata.

### 2. Design and Wire the Hardware System
Choose the cleanest path for the user's request:
- **Fast turnkey build from prompt:** Call `labwired_hardware` with the full functional requirement, target board (if specified), and acceptance behavior. It orchestrates schematic wiring, firmware authoring, compilation, and simulation verification in one pass.
- **Explicit step-by-step control:** Create or load a project with `labwired_project_create` / `labwired_project_get`, wire peripherals cleanly with `labwired_connect_interfaces`, and inspect the active topology with `labwired_describe`.

### 3. Validate Electrical Integrity (Unlimited on Free Tier)
Before compiling or running firmware, call `labwired_validate` on the diagram:
- Fix every error: missing GND/VCC reference, bus pull-up omissions (I2C SDA/SCL), voltage-level mismatches (3.3 V vs 5 V), shorted rails, or invalid pin bindings.
- Re-run `labwired_validate` until `errors` is empty.

### 4. Compile Firmware
- Upload or edit firmware sources with `labwired_put_source` (or `labwired_project_edit_firmware`).
- Compile with `labwired_compile` targeting the board from the diagram.
- If compilation fails, inspect compiler diagnostics, patch the source, and re-compile until an ELF / flash artifact is produced.

### 5. Run Deterministic Verification (`labwired_verify`)
Never claim hardware "works" without execution proof:
- Call `labwired_verify` with explicit, falsifiable oracle assertions matching the specification:
  - `uart_contains` / `uart_regex` for protocol banners, sensor telemetry, or CLI output
  - `gpio_edges` (`{ pin, min_edges }`) for LEDs, PWM, chip-select, or interrupts
  - `motor_speed_reached` for actuator/motor loops
  - `stimuli` / `uart_injections` for button presses, sensor changes, or host commands
- If `labwired_verify` returns a pending `job_id`, poll `labwired_verify` with that `job_id` (polling does not consume additional daily build quota).
- Inspect any failing assertion or verification gap, fix the circuit or firmware, and re-run until `verdict: "pass"`.

### 6. Price Real BOM & Stock
- Call `labwired_price` on the validated diagram to fetch real distributor pricing, MPNs, and live stock availability.
- Flag any out-of-stock or obsolete parts and propose in-stock alternatives from `labwired_part_search`.

### 7. Optional: PCB Layout & KernelCAD 3D Enclosure
When the user asks for a physical board or enclosure:
- Generate and route the KiCad PCB with `labwired_pcb_auto` (or `labwired_pcb_create` -> `labwired_pcb_place` -> `labwired_pcb_route`), then verify zero DRC violations with `labwired_pcb_inspect`.
- Discover parametric 3D CAD tools with `labwired_cad_tools` and generate a board-matched 3D enclosure with `labwired_cad_call` (or inspect assembly via `labwired_device_assembly`).

### 8. Deliver a Live Twin Link
- Call `labwired_project_publish` (or `labwired_lab`) to produce a live, interactive LabWired URL the user can open, simulate, and inspect in the browser.

## Reporting Checklist
Always return:
1. Verification verdict (`pass`/`fail`) with observed UART/GPIO/bus evidence.
2. Live LabWired playground link (if created/published).
3. BOM total cost & part count from `labwired_price`.
4. PCB DRC / 3D enclosure status (if requested).
