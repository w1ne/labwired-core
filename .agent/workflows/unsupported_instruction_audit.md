---
description: Run code-driven unsupported-instruction audit and produce implementation backlog artifacts
---

# Unsupported Instruction Audit (Agent Job)

Use this job when simulator behavior is suspicious or when onboarding a new MCU/board target.

This is a runtime validator (code-driven), not a datasheet/schema validator.
It executes real firmware and reports unsupported instructions observed in the run.

## Command

Run from repository root:

```bash
core/scripts/unsupported_instruction_audit.sh \
  --firmware core/target/thumbv7m-none-eabi/release/<firmware-demo-crate> \
  --system core/configs/systems/<board>.yaml \
  --max-steps 200000 \
  --out-dir core/out/unsupported-audit/<board>
```

Optional strict mode (CI gate):

```bash
core/scripts/unsupported_instruction_audit.sh \
  --firmware core/target/thumbv7m-none-eabi/release/<firmware-demo-crate> \
  --system core/configs/systems/<board>.yaml \
  --fail-on-unsupported
```

## Artifacts

The job writes:

1. `report.md` (human summary)
2. `unknown_thumb16_summary.tsv`
3. `unhandled_thumb32_summary.tsv`
4. `unknown_riscv_summary.tsv`
5. raw simulator logs (`simulator.log`, `simulator.clean.log`)

## Agent Follow-up

1. Prioritize unsupported instructions by frequency.
2. Implement decoder/executor support with tests.
3. Re-run this job until unsupported count reaches `0` (or explicitly document deferred instructions).
