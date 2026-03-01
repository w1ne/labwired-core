# AGENTS.md

Repository-level operating manual for AI coding agents working in `labwired`.

## 1) Mission Context (Read First)

- LabWired is a deterministic firmware simulation platform for CI/debug/agent workflows.
- Current strategy is **Protocol-First**:
  - from "Simulator-as-a-Service" framing
  - to **Simulation Protocol** framing (deterministic execution contract + artifacts + interoperability)
- Core simulator and tooling are open source.

Primary strategy references:
- `docs/plan.md`
- `docs/README.md`
- `docs/AGENT_INTERFACE.md`
- `docs/VS_CODE_UI_DEMO_CHECKLIST.md`

## 2) Canonical Documentation Map

Start here:
- `README.md` (repo entrypoint)
- `DEVELOPMENT.md` (build/test flows)
- `core/README.md` (engine-specific entrypoint)
- `core/docs/index.md` (core docs hub)

Architecture and runtime behavior:
- `core/docs/architecture.md`
- `core/docs/debugging.md`
- `core/docs/reference_client_flows.md`
- `core/docs/demos.md`

Release and quality gates:
- `core/docs/release_strategy.md`
- `CHANGELOG.md`

Board onboarding and peripheral contribution:
- `core/docs/board_onboarding_playbook.md`
- `core/docs/CONTRIBUTING_PERIPHERALS.md`

Roadmap and positioning:
- `docs/plan.md`
- `docs/vision/AGENT_FIRST_PLATFORM.md`
- `docs/vision/VISION_COMPLETION_GAPS.md`

## 3) Repo Layout and Ownership

- `core/`: simulator engine, CLI, DAP, configs, firmware fixtures, core tests.
- `vscode/`: VS Code/Antigravity extension for human observer workflows.
- `docs/`: strategy, roadmap, runbooks, pitch/demo docs.
- `marketing/`: external-facing comparison/blog positioning.
- `core/scripts/`: helper scripts for demos, audits, and automation.

Operational guidance:
- Prefer implementation and validation in `core/` for simulator behavior changes.
- Keep extension changes in `vscode/` when task is IDE/debug UX.
- Keep narrative/positioning updates in `docs/` and `marketing/`.

## 4) Standard Development Commands

From repo root:

```bash
# Core build/test/lint (exclude all embedded firmware crates that require cross-compilation targets)
cd core
EXCLUDES="--exclude firmware-armv6m-hello --exclude firmware-stm32f103-blinky --exclude firmware-stm32f103-uart --exclude firmware-armv6m-ci-fixture --exclude firmware-armv7m-benchmark --exclude firmware-f401-demo --exclude firmware-h563-demo --exclude firmware-h563-fullchip-demo --exclude firmware-h563-io-demo --exclude firmware-hil-showcase --exclude firmware-nrf52832-demo --exclude firmware-rp2040-pio-onboarding --exclude firmware-rv32i-ci-fixture --exclude firmware-rv32i-hello"
cargo build --workspace $EXCLUDES
cargo test --workspace $EXCLUDES
cargo clippy --workspace $EXCLUDES -- -D warnings
cargo fmt --all -- --check
```

Run simulator:

```bash
cd core
cargo run -p labwired-cli -- --firmware path/to/firmware.elf --system path/to/system.yaml
```

CI runner mode:

```bash
cd core
cargo build --release -p labwired-cli
./target/release/labwired test --script examples/ci/uart-ok.yaml --output-dir ../out/artifacts --no-uart-stdout
```

Unsupported-instruction audit:

```bash
cd core
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/<firmware-crate> \
  --system configs/systems/<board>.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/<board>
```

VS Code extension:

```bash
cd vscode
npm install
npm run compile
```

## 5) Agent Execution Rules (All Tasks)

1. Prefer existing docs and scripts over ad-hoc behavior.
2. Keep changes scoped; do not mix unrelated edits in one commit.
3. Validate touched paths with executable commands.
4. Report exact commands run and concrete evidence (logs/artifacts/paths).
5. Do not claim completion without passing checks relevant to changed components.

## 5.1) Pre-Flight Validation (Hallucination Prevention)

Before starting any task, the agent MUST:
1. **Locate Source Evidence:** Confirm the task exists in a documented file with a `[TODO:AI]` or `[OPENCLAW]` tag.
2. **Path Audit:** Verify that all paths mentioned in the task actually exist and match the current repo state.
3. **Negative Path Definition:** Explicitly state what constitutes a "Task Invalid" state (e.g., "If the bug is not reproducible after 3 attempts, the task is marked as hallucination").
4. **Plan Approval:** For non-trivial tasks, the proposed plan must be written to a temporary artifact and verified by a secondary reasoning call (or human) before execution.

## 6) Board Onboarding SOP (Mandatory for New MCU/Board Work)

Apply this checklist whenever the task is adding/simulating a new MCU/board target.

### Procedure (Phase Gates)

1. `P0 - Source grounding`
   - Read `core/docs/board_onboarding_playbook.md`.
   - Collect authoritative vendor docs (CMSIS device header + board BSP header).
   - Record source links for final report.
2. `P1 - Engine fit`
   - Map board requirements to supported peripheral types.
   - Select minimal deterministic bring-up subset (`rcc + gpio + uart + systick` by default).
3. `P2 - Implementation`
   - Add chip descriptor: `core/configs/chips/<chip>.yaml`.
   - Add board/system manifest: `core/configs/systems/<board>.yaml`.
   - Add/adapt minimal smoke firmware crate.
   - Add tests for engine behavior changes.
4. `P3 - Example docs package`
   - Add `core/examples/<board>/` package with required docs.
   - Ensure commands in docs are executable as written.
5. `P4 - Validation`
   - Run test/build/run commands.
   - Confirm PC/SP initialization and deterministic UART smoke output.
   - Run unsupported instruction audit.
6. `P5 - Report`
   - Provide files changed, commands run, runtime evidence, and source links.

### Required Deliverables

1. `core/configs/chips/<chip>.yaml`
2. `core/configs/systems/<board>.yaml`
3. smoke firmware crate (new or adapted)
4. `core/examples/<board>/system.yaml`
5. `core/examples/<board>/README.md`
6. `core/examples/<board>/REQUIRED_DOCS.md`
7. `core/examples/<board>/EXTERNAL_COMPONENTS.md`
8. `core/examples/<board>/VALIDATION.md`

### Validation Template (Run from `core/`)

```bash
cargo test -p labwired-core <new_or_updated_test_name> -- --nocapture
cargo build -p <firmware-demo-crate> --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7m-none-eabi/release/<firmware-demo-crate> \
  --system examples/<board>/system.yaml \
  --max-steps 32
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/<firmware-demo-crate> \
  --system configs/systems/<board>.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/<board>
```

### Completion Criteria

A board onboarding task is complete only when all are true:

1. `labwired-cli` runs firmware with the new system manifest.
2. Reset initializes PC/SP correctly.
3. Expected UART smoke output is observed.
4. New/updated tests pass for touched behavior.
5. Unsupported-instruction audit report is generated and reviewed.

### Final Report Requirements

1. List all added/edited files.
2. Include exact validation commands run.
3. Include runtime evidence (PC/SP init + UART output).
4. Link source references (memory map + board pin mapping).
5. Include unsupported-instruction audit summary (counts + artifact paths).

### Stop Conditions

Do not mark onboarding complete if any is true:

1. No runnable example in `core/examples/<board>/`.
2. Required docs missing from example folder.
3. Validation commands not executed.
4. Final report missing runtime evidence or source links.
5. Unsupported-instruction audit not run or artifacts missing.

## 7) Release Readiness Scoreboard

Use this to track progress toward the v0.1.0 "VC-Ready" public release.

| Feature | Status | Goal |
| :--- | :--- | :--- |
| **Core Determinism Proof** | 🟡 Partial | Periodic golden-board validation suite. |
| **Agentic Loop (AI Foundry)** | 🟡 In-Progress | Zero-touch datasheet -> model path. |
| **Professional Debugging** | 🟢 Ready | Timeline, registers, memory inspector. |
| **CI Integration** | 🟢 Ready | `labwired test` with machine artifacts. |
| **Compatibility Matrix** | 🟢 Ready | Documented Tier-1 vs Tier-2 support. |
| **Foundry (Cloud Service)** | ⚪ Not Started | Multi-tenant hosted execution. |
