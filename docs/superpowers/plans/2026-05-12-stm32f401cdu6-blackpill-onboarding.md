# STM32F401CDU6 Black Pill Onboarding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add STM32F401CDU6 Black Pill as a LabWired board target with simulation trace validation and ST-Link-only hardware sanity evidence.

**Architecture:** Reuse the existing STM32F401 simulation support and F401 demo firmware path, adding Black Pill-specific chip memory sizing, board IO, example scripts, catalog metadata, docs, and playground bundling. Physical validation is limited to USB/ST-Link visibility; UART evidence comes from deterministic simulation artifacts.

**Tech Stack:** Rust/Cargo LabWired core CLI, YAML board/chip manifests, Vitest playground bundling tests, Linux USB enumeration.

---

### Task 1: Add Bundled Config Regression Coverage

**Files:**
- Modify: `packages/playground/src/bundled-configs.test.ts`

- [ ] **Step 1: Write the failing test**

Add assertions for a `stm32f401cdu6-blackpill` board config:

```ts
const blackPill = BOARD_CONFIGS.find((config) => config.boardId === 'stm32f401cdu6-blackpill');

expect(blackPill).toBeDefined();
expect(blackPill?.chipYaml).toContain('name: "stm32f401cdu6"');
expect(blackPill?.chipYaml).toContain('size: "384KB"');
expect(blackPill?.systemYaml).toContain('led_pc13');
expect(blackPill?.systemYaml).toContain('active_high: false');
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
npm --prefix packages/playground test -- --run src/bundled-configs.test.ts
```

Expected: FAIL because the Black Pill board is not in `BOARD_CONFIGS`.

### Task 2: Add Black Pill Manifests and Playground Bundle

**Files:**
- Create: `core/configs/chips/stm32f401cdu6.yaml`
- Create: `core/configs/systems/stm32f401cdu6-blackpill.yaml`
- Modify: `packages/playground/src/bundled-configs.ts`

- [ ] **Step 1: Add chip manifest**

Create an STM32F401CDU6 descriptor with 384KB flash, 96KB RAM, the executable LabWired models available for the STM32F401xB/xC map, and safe stubs for mapped blocks that do not yet have dedicated behavior.

- [ ] **Step 2: Add system manifest**

Create a board system with active-low LED on PC13 and a user button entry on PA0 for common Black Pill variants.

- [ ] **Step 3: Bundle the board in playground**

Import both manifests and add a `BOARD_CONFIGS` entry with `boardId: 'stm32f401cdu6-blackpill'`.

- [ ] **Step 4: Run test to verify it passes**

Run:

```bash
npm --prefix packages/playground test -- --run src/bundled-configs.test.ts
```

Expected: PASS.

### Task 3: Add Core Onboarding Catalog and Example

**Files:**
- Create: `core/configs/chips/onboarding/stm32f401cdu6-blackpill.yaml`
- Create: `core/configs/systems/onboarding/stm32f401cdu6-blackpill.yaml`
- Create: `core/configs/onboarding/stm32f401cdu6-blackpill.yaml`
- Create: `core/examples/stm32f401cdu6-blackpill/system.yaml`
- Create: `core/examples/stm32f401cdu6-blackpill/trace-smoke.yaml`
- Create: `core/examples/stm32f401cdu6-blackpill/README.md`
- Create: `core/examples/stm32f401cdu6-blackpill/VALIDATION.md`
- Create: `core/crates/firmware-f401cdu6-blackpill-demo/`
- Modify: `core/Cargo.toml`

- [ ] **Step 1: Add onboarding chip/system/catalog triplet**

Mirror the root chip/system manifests into the onboarding catalog shape and set validation method to local simulation trace evidence.

- [ ] **Step 2: Add example system and trace smoke script**

Use the Black Pill system manifest and F401 demo firmware; assert `uart_contains: "OK"` and `expected_stop_reason: max_steps`.

- [ ] **Step 3: Add runbook docs**

Document that ST-Link USB visibility is hardware sanity only and that trace-backed simulation is the onboarding acceptance gate.

### Task 4: Validate and Collect Artifacts

**Files:**
- Output artifacts under: `core/out/stm32f401cdu6-blackpill/`

- [ ] **Step 1: Validate YAML assets**

Run:

```bash
cargo run -q -p labwired-cli -- asset validate --chip configs/chips/onboarding/stm32f401cdu6-blackpill.yaml
cargo run -q -p labwired-cli -- asset validate --system configs/systems/onboarding/stm32f401cdu6-blackpill.yaml
```

Expected: both exit `0`.

- [ ] **Step 2: Build firmware**

Run:

```bash
cargo build -p firmware-f401cdu6-blackpill-demo --release --target thumbv7em-none-eabi
```

Expected: exit `0`.

- [ ] **Step 3: Run trace smoke**

Run:

```bash
cargo run -q -p labwired-cli -- test --script examples/stm32f401cdu6-blackpill/trace-smoke.yaml --output-dir out/stm32f401cdu6-blackpill/trace-smoke --no-uart-stdout --trace --trace-max 128
```

Expected: exit `0`, with `result.json`, `snapshot.json`, `trace.json`, and `uart.log`.

- [ ] **Step 4: Run direct JSON/VCD simulation**

Run:

```bash
cargo run -q -p labwired-cli -- --firmware target/thumbv7em-none-eabi/release/firmware-f401cdu6-blackpill-demo --system configs/systems/stm32f401cdu6-blackpill.yaml --max-steps 32 --json --vcd out/stm32f401cdu6-blackpill/blackpill.vcd
```

Expected: exit `0`, JSON reports `status: "finished"` and VCD file exists.

- [ ] **Step 5: Confirm ST-Link USB presence**

Run:

```bash
lsusb
```

Expected: output includes `0483:3748 STMicroelectronics ST-LINK/V2`.
