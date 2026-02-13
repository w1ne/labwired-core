# Board Onboarding Playbook (Agent Runbook)

This runbook documents how to add a new board target to the LabWired core engine in a way that is reliable for future agents.

## Standard Procedure (Typical Onboarding Task)

Use this as the default flow for every new board onboarding request.

### Phase 0: Source Grounding

Inputs:
- board/MCU name from user

Actions:
1. Gather primary sources (CMSIS device header + board BSP header).
2. Capture source URLs in working notes.

Exit criteria:
- base addresses, IRQs, and board COM/LED mapping are traceable to primary docs.

### Phase 1: Engine Fit and Scope

Actions:
1. Compare target needs against currently supported peripheral `type` values.
2. Select minimal bring-up scope when full silicon coverage is not feasible.

Exit criteria:
- explicit list of modeled vs deferred peripherals.

### Phase 2: Configuration and Firmware Implementation

Actions:
1. Add chip descriptor (`core/configs/chips/<chip>.yaml`).
2. Add system manifest (`core/configs/systems/<board>.yaml`).
3. Add/adapt smoke firmware crate for deterministic UART output.
4. Add/adjust engine tests if runtime behavior changed.

Exit criteria:
- code builds and configuration loads.

### Phase 3: Example Folder Documentation Pack

Create `core/examples/<board>/` with:

1. `system.yaml`
2. `README.md`
3. `REQUIRED_DOCS.md`
4. `EXTERNAL_COMPONENTS.md`
5. `VALIDATION.md`

Exit criteria:
- docs are complete and commands are executable as written.

### Phase 4: Validation

Actions:
1. Run tests.
2. Build smoke firmware.
3. Run simulator with example-local `system.yaml`.

Exit criteria:
- PC/SP initialize correctly
- UART smoke output observed
- test suite changes pass

### Phase 5: Handoff Report

Include:
1. files changed
2. exact commands run
3. key runtime output evidence
4. source links used

## Goal

Given a real MCU board (example: NUCLEO-H563ZI), produce:

1. A chip descriptor in `core/configs/chips/`
2. A board/system manifest in `core/configs/systems/`
3. A minimal firmware smoke test that proves reset + UART path works
4. A deterministic validation command sequence

## Engine Reality Check (Do This First)

Before modeling a board, confirm currently supported peripheral `type` values in `SystemBus::from_config`:

- `uart`
- `systick`
- `gpio`
- `rcc`
- `timer`
- `i2c`
- `spi`
- `exti`
- `afio`
- `dma`
- `adc`
- `declarative` / `strict_ir`

If the board has many unsupported blocks, pick a minimal subset that can boot and demonstrate value (typically `rcc + gpio + uart + systick`).

## Universal Peripheral Config Structure

Use a single config key for built-in peripheral variants:

- `config.profile`: profile selector for modeled register map/behavior

Current built-in profile-enabled peripherals:

- `gpio`: `stm32f1`, `stm32v2`
- `uart`: `stm32f1`, `stm32v2`
- `rcc`: `stm32f1`, `stm32v2`

Backward compatibility:

- `config.register_layout` is still accepted as a legacy alias for `config.profile`.

## Required Source Material

Use primary vendor sources:

1. MCU CMSIS device header for memory map and IRQ numbers
2. Board BSP header for LED/UART/button pin mapping
3. Board/MCU product pages for traceability

For STM32H563ZI demo:

- MCU header: `stm32h563xx.h` (`cmsis-device-h5`)
- Board BSP: `stm32h5xx_nucleo.h` (`stm32h5xx-nucleo-bsp`)

## Implementation Steps

### 1) Create chip descriptor

Add `core/configs/chips/<chip>.yaml` with:

- `flash.base` and `flash.size`
- `ram.base` and `ram.size`
- only supported peripherals with correct base addresses/IRQs

H563 example file: `core/configs/chips/stm32h563.yaml`

### 2) Create board/system manifest

Add `core/configs/systems/<board>.yaml` pointing at the chip descriptor.

H563 example file: `core/configs/systems/nucleo-h563zi-demo.yaml`

### 3) Ensure reset vector fetch works

Important: Cortex-M reset reads vectors from address `0x00000000`.
If flash is at `0x08000000`, engine must support boot aliasing or firmware must be linked at `0x00000000`.

For H563, boot alias support was implemented in:

- `core/crates/core/src/bus/mod.rs`

This maps reads/writes from `0x00000000..flash_size` to `flash.base`.

### 4) Add minimal firmware smoke target

Create a tiny `no_std` firmware that:

1. has a valid vector table
2. writes `OK\n` to the mapped UART TX register
3. loops forever

H563 example crate:

- `core/crates/firmware-h563-demo/`

### 5) Register crate in workspace

Add the new demo crate path in `core/Cargo.toml` `[workspace].members`.

## Validation Commands

Run from `core/`:

```bash
cargo test -p labwired-core test_flash_boot_alias_read_and_write -- --nocapture
cargo build -p firmware-h563-demo --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7m-none-eabi/release/firmware-h563-demo \
  --system configs/systems/nucleo-h563zi-demo.yaml \
  --max-steps 32
```

Expected outcome:

- simulator starts successfully
- PC initializes to flash region (`0x08000000` range)
- UART output contains `OK`

## Common Failure Modes

1. PC stays at `0x00000000` or faults immediately.
Cause: reset vectors are not reachable at `0x00000000`.
Fix: add boot alias handling or link vector table at `0x00000000`.

2. Simulator loads but no UART output appears.
Cause: wrong UART base address/instance for board COM port.
Fix: verify BSP mapping (`COM1_UART`, TX/RX pin macros) and chip base address.

3. Memory violations right after reset.
Cause: incorrect `flash.base`, `ram.base`, or RAM size.
Fix: re-check CMSIS header base/size constants.

4. Build succeeds but firmware never reaches user code.
Cause: invalid minimal linker script/vector table.
Fix: verify initial SP is in RAM range and reset handler uses Thumb bit (`Reset + 1`).

## Agent Decision Rules

When onboarding a new board, follow these rules:

1. Prefer correctness over coverage: model fewer peripherals accurately.
2. Use vendor headers as source of truth for addresses and IRQs.
3. Always include a deterministic smoke firmware and command log.
4. Add at least one unit test for any engine behavior change.
5. Document assumptions and known gaps directly in the chip YAML comments.

## Handoff Checklist

Before finishing, ensure these are all done:

1. `core/configs/chips/<chip>.yaml` added and commented with source references.
2. `core/configs/systems/<board>.yaml` added.
3. Demo firmware crate added (or existing fixture adapted).
4. Engine tests updated if runtime behavior changed.
5. `core/examples/<board>/` documentation pack exists and is complete.
6. Commands and observed output included in final response.

## Known Gaps for H563 Demo

- Peripheral behavior is still generic (not H5 register-accurate yet).
- Board-level LED/button electrical behavior is not modeled.
- This is a bring-up profile for demo/agent workflow, not final silicon-fidelity.
