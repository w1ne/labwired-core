# AGENTS.md

Repository-level instructions for coding agents working in `labwired`.

## Scope

Apply this checklist whenever the user asks to add or simulate a new MCU/board target.

## Typical Task Type: Board Onboarding

Treat board onboarding as a standard operating procedure (not an ad-hoc task).
Follow the phases in order and do not skip validation/documentation phases.

## Procedure (Phase Gates)

1. `P0 - Source grounding`
   - Read `core/docs/board_onboarding_playbook.md`.
   - Collect authoritative vendor docs (CMSIS device header + board BSP header).
   - Record links for final report.
2. `P1 - Engine fit`
   - Map board requirements to currently supported peripheral types.
   - Select a minimal viable subset for deterministic bring-up (`rcc + gpio + uart + systick` by default).
3. `P2 - Implementation`
   - Add chip descriptor in `core/configs/chips/`.
   - Add board/system manifest in `core/configs/systems/`.
   - Add or adapt minimal smoke firmware crate.
   - Add tests for any engine behavior change.
4. `P3 - Example docs package`
   - Add `core/examples/<board>/` with all required docs listed below.
   - Ensure commands in docs are executable as written.
5. `P4 - Validation`
   - Run test/build/run commands.
   - Confirm PC/SP initialization and deterministic UART smoke output.
   - Run code-driven unsupported-instruction audit (`core/scripts/unsupported_instruction_audit.sh`).
6. `P5 - Report`
   - Provide files changed, commands run, key runtime evidence, and source links.

## Required Deliverables for Each Board

1. `core/configs/chips/<chip>.yaml`
2. `core/configs/systems/<board>.yaml`
3. smoke firmware (new or adapted crate)
4. `core/examples/<board>/system.yaml`
5. `core/examples/<board>/README.md`
6. `core/examples/<board>/REQUIRED_DOCS.md`
7. `core/examples/<board>/EXTERNAL_COMPONENTS.md`
8. `core/examples/<board>/VALIDATION.md`

## Board Onboarding Checklist (Short)

When adding support for a new MCU board target, follow this sequence:

1. Read the full playbook: `core/docs/board_onboarding_playbook.md`.
2. Use primary vendor sources only:
   - MCU CMSIS header (memory map + IRQs)
   - Board BSP header (LED/UART/button mapping)
3. Fit the target to currently supported engine peripheral types first (`uart`, `gpio`, `rcc`, `systick`, etc.).
4. Add chip config: `core/configs/chips/<chip>.yaml`.
5. Add board/system config: `core/configs/systems/<board>.yaml`.
6. Ensure reset vector path works (boot alias or linker placement).
7. Add minimal smoke firmware that outputs deterministic UART text (for example, `OK\n`).
8. Add at least one test for any engine-level behavior change.
9. Validate with executable commands and capture expected output in your final note.
10. Run unsupported-instruction audit and attach generated report artifacts.

## Validation Template (Use As-Is, Then Adapt)

Run from `core/`:

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

## Completion Criteria

A board onboarding task is complete only when:

1. `labwired-cli` runs firmware with the new system manifest.
2. Reset initializes PC/SP correctly.
3. Expected UART smoke output is observable.
4. New/updated tests pass for touched behavior.
5. Unsupported-instruction audit report is generated and reviewed.

## Reporting Requirements

In the final response:

1. List all added/edited files.
2. Include the exact validation commands run.
3. Include key runtime evidence (PC/SP init + UART smoke output).
4. Link source references used for memory map and board pin mapping.
5. Include unsupported-instruction audit summary (counts + artifact paths).

## Stop Conditions

If any of the following is true, onboarding is not complete:

1. No runnable example exists in `core/examples/<board>/`.
2. Required docs are missing from the example folder.
3. Validation commands were not executed.
4. Final report does not include runtime evidence and source links.
5. Unsupported-instruction audit was not run (or report artifacts are missing).
