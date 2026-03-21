[← Back to Hub](../../README.md)

# Vision Completion Gaps (Agent-First Platform)

This document captures what is still missing to finish the vision in `docs/vision/AGENT_FIRST_PLATFORM.md`.

## Scope

Vision target:
- Phase 1: Foundation
- Phase 2: Agentic Loop
- Phase 3: Foundry

Completion standard:
- "Vision complete" means all three phases are operational, not only documented.

## Current Baseline (What Already Exists)

- Deterministic headless simulation core and CI runner flow (`labwired test`) with machine-readable artifacts (`result.json`, `snapshot.json`, `junit.xml`).
- Agent-oriented tooling exists in CLI (`asset validate`, `asset import-svd`, `asset list-chips`) and Python side (`ai/labwired_ai`).
- Unsupported-instruction audit workflow exists (`core/scripts/unsupported_instruction_audit.sh`).
- Human observer path exists via VS Code extension (timeline/register/memory features are present at basic level).

## Gap Status (Updated 2026-03-21)

For a detailed item-by-item tracker, see [SCOREBOARD.md](./SCOREBOARD.md).

## 1) Foundation Hardening — LARGELY CLOSED

**Closed:**
- Trace-level determinism proof (SHA-256 hash comparison across 5 runs) runs as `determinism-proof` CI gate.
- Auto-generated compatibility matrix (`core/scripts/generate_compat_matrix.py`) uploaded as CI artifact per build.
- BTreeMap-based trace serialization eliminates non-deterministic JSON output.

**Remaining:**
- Golden-reference board validation (periodic real hardware baseline) is still open in planning.
- Run artifact standardization is not fully unified across all modes (interactive CLI vs test mode vs AI workflows).

## 2) Agentic Loop — LARGELY CLOSED

**Closed:**
- End-to-end `auto-ingest` orchestrator with ingest → IR convert → verify → LLM-assisted retry loop (max 3x).
- Confidence scoring with auto-approve threshold (>= 0.9 pass rate).

**Remaining:**
- Tier-1 target zero-touch validation coverage not yet confirmed end-to-end.
- AIPi contract versioning and backward-compatibility policy not formalized.
- Advanced timing semantics and cross-peripheral side effects not yet in synthesis pipeline.

## 3) Metering Economy — LARGELY CLOSED

**Closed:**
- Per-operation type tracking (`op_type` column) and simulation minutes (`sim_minutes` column) in Foundry DB.
- Usage breakdown endpoint (`GET /v1/account/usage/breakdown`) for per-operation reporting.
- Python SDK telemetry auto-exports to Foundry when `LABWIRED_FOUNDRY_URL` is set.

**Remaining:**
- COGS tracking per run and pricing instrumentation for billing-grade monetization.

## 4) Foundry Multi-Tenancy — LARGELY CLOSED

**Closed:**
- Organization model with `organizations` and `org_members` tables.
- RBAC middleware (admin > developer > viewer) on org-scoped endpoints.
- Audit logging with `audit_log` table and `GET /v1/account/org/{id}/audit` endpoint.
- Org management API (`POST/GET /v1/account/org`, member management).

**Remaining:**
- Secure cloud execution fabric (scheduler, isolation boundary, fleet management).
- SSO integration beyond Clerk.
- Compliance evidence pipeline / retention policy.

## 5) Human Observer (VS Code) — LARGELY CLOSED

**Closed:**
- Conditional breakpoints with expression evaluation (register comparisons, hex/decimal).
- Data breakpoints (watchpoints) triggering on memory writes.
- Watch expressions with memory dereference (`*(0xADDR)`) and register arithmetic.
- Live hover provider for register names and hex addresses.
- Improved Thumb-2 32-bit disassembly view with source line correlation.

**Remaining:**
- Reverse debugging and RTOS-awareness.
- One-click "open project → matched simulator" flow not fully automated.

## 6) Documentation & Delivery — CLOSED

**Closed:**
- Vision Completion Scoreboard (`docs/strategy/vision/SCOREBOARD.md`).
- Getting Started tutorial (`docs/tutorials/getting-started.md`).
- CI Integration tutorial (`docs/tutorials/ci-integration.md`).
- Documentation hub links updated.

## Definition of Done for Vision Completion

The vision is complete when all conditions below are true:

1. Foundation:
- Stable build/test baseline in `main`.
- Determinism evidence published per release (reproducibility + golden-board checks).
- Compatibility matrix published and updated each release.

2. Agentic Loop:
- Datasheet/SVD -> model -> simulation -> validation runs with no manual edits for at least Tier-1 target set.
- AIPi contract is versioned, tested, and backward-compatibility policy enforced.

3. Metered Economy:
- Quota enforcement and billing-grade usage accounting are active in runtime services.
- Organization-level usage visibility and limits are enforced.

4. Foundry:
- Hosted API can create/run/delete simulation jobs in isolated environments.
- Tenancy, RBAC, audit logs, and compliance evidence exports are operational.

5. Human Observer:
- One-click IDE lifecycle flow is stable.
- Core advanced debug features required by roadmap are shipped.

## Suggested Execution Priority (Highest First)

1. Ship determinism proof + compatibility matrix as release gates.
2. Close zero-touch Agentic Loop for Tier-1 targets.
3. Implement metering backend + tenancy primitives.
4. Build hosted Foundry control plane and execution service.
