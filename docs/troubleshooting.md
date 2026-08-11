# Troubleshooting

Common failures when running LabWired. Fix the twin or the firmware — do not ignore a red result.

---

## Simulation

### Memory violation (`MemoryAccessViolation` / `MemoryViolation`)

**Meaning:** firmware touched an address with no flash, RAM, or modeled peripheral.

**Do:**

1. Check flash/RAM in `configs/chips/<chip>.yaml`  
2. Check missing peripherals in the chip or system YAML  
3. Align the firmware linker script with the chip map  
4. See the board ✅ / ⚠️ / ❌ matrix — you may have hit a stub  

### Decode / illegal instruction

**Meaning:** CPU executed invalid opcodes (often bad entry point).

**Do:**

1. Check the vector table / reset handler  
2. Confirm the firmware loaded (try `labwired run` with a known-good example)  
3. On Cortex-M, Thumb entry addresses are odd (LSB = 1)  

### Empty or missing serial

**Meaning:** firmware never wrote the UART you are watching, or wiring is wrong.

**Do:**

1. Match UART instance and pins to the system YAML  
2. Confirm clocks enabled the UART block  
3. Read `uart.log` from `labwired test` output dir  

### Max steps / no progress

**Meaning:** simulation hit the step budget or a tight infinite loop.

**Do:**

1. Raise `max_steps` in the test script if boot is long  
2. Find the loop with `--trace`  
3. Fix firmware waits that never complete on the twin (PLL, flags on unmodeled bits)  

### DAP / GDB connection refused

**Do:**

1. Start LabWired with GDB enabled as in [GDB integration](gdb_integration.md)  
2. Match the port in your debugger config (often `3333`)  

---

## CI

| Symptom | Fix |
|---------|-----|
| Different result locally vs CI | Pin the same `LABWIRED_VERSION` / action `version` |
| Action cannot download CLI | Check release tag and network; pin a known-good version |
| JUnit missing | Pass `--junit` / action `output-dir` and upload artifacts `if: always()` |

See [CI integration](ci_integration.md).

---

## Parts and diagrams

| Symptom | Fix |
|---------|-----|
| Device not found | Type id must match `configs/devices/` / catalog; `labwired_describe` |
| I²C NACK forever | Address, bus id, and wiring; WHO_AM_I first |
| Agent invents pins | Force `labwired_describe` before wire |

See [Onboard a part](howto/onboard-part.md).

---

## Next

- [Run firmware](getting_started_firmware.md)
- [Fidelity](fidelity.md)
- [Board playbook](board_onboarding_playbook.md)
