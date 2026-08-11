# Board onboarding playbook

Add a **new MCU or board** to LabWired: chip YAML, system YAML, smoke firmware, and proof.

For **sensors, SPI chips, and actuators**, start with [Onboard a part](howto/onboard-part.md) instead.

---

## Gold reference

**NUCLEO-L476RG** is the end-to-end example. Read these with this playbook:

| Asset | Path |
|-------|------|
| Example README | [`examples/nucleo-l476rg/README.md`](../examples/nucleo-l476rg/README.md) |
| Validation trail | [`examples/nucleo-l476rg/VALIDATION.md`](../examples/nucleo-l476rg/VALIDATION.md) |
| Board docs | [`docs/boards/nucleo-l476rg.md`](boards/nucleo-l476rg.md) |
| Chip YAML | [`configs/chips/stm32l476.yaml`](../configs/chips/stm32l476.yaml) |
| System YAML | [`configs/systems/nucleo-l476rg.yaml`](../configs/systems/nucleo-l476rg.yaml) |

Copy structure from a nearby family member when the silicon is similar (STM32 F4/L4/H5, nRF, ESP, RP).

---

## 1. Prerequisites

1. MCU reference manual (memory map, peripherals)
2. Datasheet (flash/RAM sizes)
3. Board user manual (LED, button, VCP UART pins)
4. Optional: CMSIS headers / SVD for IRQs and bases

---

## 2. Fit check

You need a minimal path: **boot + something observable** (usually UART print or GPIO toggle).

Typical first peripherals:

- Clock / reset (`rcc` or vendor equivalent)
- GPIO
- UART (or USB-serial on chips that use it)
- SysTick or a timer if the firmware needs time

If the board **cannot** boot without USB, Ethernet, or complex power sequencing, plan that work first or pick a simpler board.

Support levels: [Target support rubric](target_support_rubric.md). Public “supported” starts at **smoke (L1)**.

---

## 3. Implementation steps

### Step 1 — Chip descriptor (`configs/chips/`)

Define flash, RAM, and peripherals with real base addresses.

```yaml
name: "STM32H563"
flash:
  base: 0x08000000
  size: "2MB"
ram:
  base: 0x20000000
  size: "640KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x44020C00
  - id: "usart3"
    type: "uart"
    base_address: 0x40004800
    irq: 55
```

Prefer existing peripheral **types** already in the engine. New on-chip blocks: [Peripheral modeling](peripherals.md).

### Step 2 — System manifest (`configs/systems/`)

Instantiate the chip and board-level wiring (VCP UART, LEDs, external parts).

```yaml
name: "NUCLEO-H563ZI"
chip: "../chips/stm32h563.yaml"
# connectors / board_io / external_devices — follow a sibling system YAML
```

Source of truth: board schematic / user manual.

### Step 3 — Smoke firmware

- Goal: init clocks + UART, print `OK\n` (or toggle LED in a way a test can see)
- Prefer vendor HAL/SDK you will ship with
- Keep the first binary small

### Step 4 — Prove with the CLI

```bash
# Build your smoke firmware with the normal toolchain for that ISA, then:

labwired run \
  --firmware path/to/smoke.elf \
  --system configs/systems/your-board.yaml

labwired test --script examples/your-board/io-smoke.yaml
```

**Success criteria (minimum):**

1. Boots (reset vector / PC sane)
2. Observable output (UART `OK` or GPIO assertion)
3. No critical unmapped accesses on the smoke path

Add `--trace` only when debugging.

### Step 5 — Document

1. `docs/boards/<id>.md` from [boards/_TEMPLATE.md](boards/_TEMPLATE.md)
2. `examples/<board>/` with README, system, and how to build
3. List known limitations honestly (✅ / ⚠️ / ❌)

---

## 4. Promote support level

| Level | Bar |
|-------|-----|
| L0 declared | Chip + system validate |
| L1 smoke | Deterministic script + artifacts |
| L2+ | CI history, audits, tier-1 peripherals |

Details: [Target support rubric](target_support_rubric.md).

---

## Next

| | |
|--|--|
| [Onboard hardware hub](howto/onboard-hardware.md) | All tracks |
| [Onboard a part](howto/onboard-part.md) | Sensors / actuators |
| [Run firmware](getting_started_firmware.md) | CLI usage |
| [CI](ci_integration.md) | Pipeline gate |
