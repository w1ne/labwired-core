# Postmortem: Unsupported Instruction Validation Gap

- Incident date: February 13, 2026 (discovered), February 14, 2026 (stabilized)
- Status: Closed
- Severity: High (simulation trust and onboarding quality risk)

## Summary

Simulation for the H563 example surfaced runtime instruction warnings (`Unknown instruction` and `Unhandled 32-bit`) that were not blocked by the existing validation path. The onboarding flow proved basic bring-up (PC/SP + UART smoke) but did not enforce instruction coverage against real firmware execution paths.

## Impact

1. Users experienced stuck stepping behavior and misleading simulator status.
2. Feature confidence dropped because "validated" flows still hit decoder gaps.
3. Engineering time was spent debugging regressions that should have been caught by CI.

## Timeline

1. **February 13, 2026**: H563 simulation issues reported (fixed cycle displays, stepping stalls, missing UART confidence).
2. **February 13, 2026**: Repro logs confirmed unsupported instruction patterns:
   - Thumb16 unknown opcode (`0x4391`)
   - Thumb32 unhandled patterns (`eb00 1010`, `fa01 f202`)
3. **February 13, 2026**: Decoder/executor support added and tests updated for impacted patterns.
4. **February 13, 2026**: Antigravity extension binary mismatch identified (old DAP binary still active), then corrected.
5. **February 14, 2026**: Code-driven unsupported-instruction audit job added and integrated into docs/CI.

## Root Cause

Primary root cause:
1. Validation process lacked a mandatory runtime opcode audit gate.

Contributing factors:
1. Existing checks emphasized configuration validity and smoke behavior, not opcode completeness.
2. Unsupported instruction events were logged but not elevated to test failure criteria.
3. Tooling version skew (installed extension binary vs local build) obscured whether fixes were active.

## Detection Gaps

1. No standardized artifact/report for unsupported instruction observations.
2. No CI job failing on unsupported instruction findings.
3. No explicit onboarding stop condition tied to runtime decode coverage.

## Corrective Actions Completed

1. Added code-driven audit script:
   - `core/scripts/unsupported_instruction_audit.sh`
2. Added agent workflow/job documentation:
   - `.agent/workflows/unsupported_instruction_audit.md`
3. Updated onboarding/agent docs to require audit execution and reporting:
   - `AGENTS.md`
   - `core/docs/board_onboarding_playbook.md`
   - `docs/AGENT_INTERFACE.md`
   - `core/examples/nucleo-h563zi/VALIDATION.md`
4. Added CI quality gate with artifact upload:
   - `.github/workflows/core-ci.yml`

## Validation Evidence

Audit runs after fixes reported zero unsupported instructions for representative firmware:

1. `core/out/unsupported-audit/nucleo-h563zi/report.md`
2. `core/out/unsupported-audit/ci-fixture/report.md`

## Preventive Controls

1. New onboarding completion criterion: unsupported-instruction audit must be run and reviewed.
2. New CI gate: fail if unsupported instructions are detected (`--fail-on-unsupported`).
3. Standardized artifacts (`report.md`, summary TSVs, raw logs) for triage and backlog creation.

## Follow-ups

1. Add optional per-architecture coverage trend tracking (count over time) in CI reports.
2. Add periodic audit runs for additional board firmware examples beyond H563.
