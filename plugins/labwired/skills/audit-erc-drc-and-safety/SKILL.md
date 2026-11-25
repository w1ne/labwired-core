---
name: audit-erc-drc-and-safety
description: Run a comprehensive pre-fabrication electrical (ERC), PCB layout (DRC), signal integrity, power/thermal margin, pin-limit, and fault-injection safety audit via LabWired MCP.
---

# Audit ERC, DRC, and Safety

Use this skill before ordering prototypes, merging hardware revisions, or approving an AI-generated circuit. It systematically audits electrical rules, PCB design rules, signal integrity, power/thermal budgets, MCU pin absolute maximums, and firmware fail-safe behavior.

## Audit Checklist & Tool Sequence

### 1. Electrical Rules Check (ERC) & Pin-Limit Audit
- Call `labwired_validate` on the diagram.
- Call `labwired_describe` and `labwired_datasheet` on the MCU and key ICs to cross-check:
  - Supply rail voltages (3.3 V vs 5 V vs 1.8 V) and logic-level compatibility across every connected net
  - Per-GPIO source/sink current limits and total package VCC/GND current limits
  - Mandatory passives: decoupling capacitors on every IC power rail, I2C pull-ups, LED current-limiting resistors, reset pull-ups, boot-strap pin states
  - Reverse-polarity, ESD, and flyback diode protection on inductive loads (relays, solenoids, motors)

### 2. Signal Integrity, Power & Thermal Margin Audit
- Call `labwired_si` to evaluate trace/signal integrity, bus loading, voltage drop, and analog margins.
- Run `labwired_pcb_simulate` (when a board layout exists) or `labwired_run` with analog instrumentation to inspect steady-state and peak current draw, regulator dropout/dissipation, and rail sag under load.
- Flag any component operating above 70% of its rated voltage, current, or thermal dissipation limit.

### 3. PCB Design Rules Check (DRC)
- If a PCB exists (generated or imported), run `labwired_pcb_inspect` and/or `labwired_pcb_validate_imported`.
- Verify:
  - Zero unconnected nets (ratsnest = 0)
  - Zero clearance, trace-width, silkscreen-over-pad, or drill/annular-ring violations
  - Adequate copper width for power rails and high-current paths
  - Clean ground return paths and crystal/RF keepout zones

### 4. Fault-Injection & Fail-Safe Firmware Audit
When firmware controls actuators, heaters, chargers, or safety-critical outputs, test failure modes with `labwired_verify`:
- Inject realistic faults via the `faults` parameter (sensor open-circuit, shorted input, I2C bus hang, power brownout).
- Assert that firmware detects the fault within its safety timeout, logs an error, and drives outputs to a safe state (e.g., heater/motor off).

## Output Format
Present the audit strictly in four sections:
1. **Blocking Issues (Must Fix Before Fab):** ERC errors, DRC errors, over-voltage/over-current violations, unsafe fault behavior.
2. **Warnings & Margin Risks:** High thermal load, marginal pull-ups, missing ESD/decoupling, tight stock.
3. **Verified Checks:** Exact outputs from `labwired_validate`, `labwired_si`, `labwired_pcb_inspect`, and `labwired_verify`.
4. **Recommended Fixes:** Concrete net/component/firmware changes ready to apply.
