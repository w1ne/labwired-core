# Catalog Validation Structure

This document defines how hardware-target validation is organized in `core`.

## Source of Truth

- `configs/onboarding/*.yaml` is the canonical source for catalog-facing target metadata.
- Validation status fields (`pass_rate`, `verified`, `validation.*`) are written in this repo by CI.

## Workflow Responsibilities

1. `core-onboarding-smoke.yml`
- Purpose: fast PR-gate smoke checks on a curated matrix of representative targets.
- Output: onboarding smoke artifacts and scoreboard.

2. `core-validate-hw-targets.yml`
- Purpose: full onboarding target sweep for catalog metadata refresh.
- Trigger: scheduled/manual and selected `main` pushes.
- Output:
  - Updated `configs/onboarding/*.yaml` with validation metadata.
  - `out/hw-target-validation/summary.json` and `summary.md` artifacts.

## Contract for Downstream Consumers

Downstream services (for example Foundry catalog ingest) should consume only:

1. onboarding manifests in this repo, and
2. validation links stored under `validation.run_url` / `validation.artifacts_url`.

They should not run separate target-validation pipelines against a forked catalog source.
