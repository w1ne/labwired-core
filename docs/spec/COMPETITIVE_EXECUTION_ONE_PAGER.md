# Competitive Execution One Pager

Date: February 22, 2026  
Audience: Daily operator / team lead  
Use this page first. Ignore deep analysis unless needed.

## 1) What Runs (Only 3 Things)

1. Top-5 coverage gate in `core/.github/workflows/core-coverage-matrix-smoke.yml`
2. Onboarding smoke KPI in `core/.github/workflows/core-onboarding-smoke.yml`
3. Core integrity gate in `core/.github/workflows/core-ci.yml`

## 2) What Good Looks Like

1. Top-5 gate: `5/5` required targets passing
2. Onboarding smoke: target run is `pass`
3. Onboarding KPI: `elapsed_seconds` is visible and not increasing week to week

## 3) Where To Look

1. Coverage: artifact `coverage-matrix-scoreboard/scoreboard.md`
2. Onboarding: artifact `onboarding-scoreboard/onboarding-scoreboard.md`
3. Per-target onboarding details: `onboarding-metrics.json`

## 4) One Action When Red

1. Find failed target id.
2. Open that target artifact logs.
3. Fix the first failing stage only (`build_cli`, `build_firmware`, or `run_smoke`).
4. Re-run pipeline.
5. Do not add new targets/features until green again.

## 5) Scope Guardrails

1. No new dashboards.
2. No new scoring models.
3. No new KPIs beyond `pass/fail` and `elapsed_seconds`.
4. Expand scope only after 2 consecutive green weeks.

## 6) Current Priority

1. Keep Top-5 green.
2. Keep onboarding smoke passing.
3. Increase functional fidelity on Top-5 paths (current: DMA copy + transfer-complete IRQ path in core).
