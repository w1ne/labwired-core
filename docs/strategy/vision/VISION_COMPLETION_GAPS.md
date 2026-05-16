[← Back to Hub](../../README.md)

# Vision Completion Gaps (Agent-First Platform)

This document captures what is still missing to finish the vision in `docs/vision/AGENT_FIRST_PLATFORM.md`.

## Scope

Vision target:
- Phase 1: Foundation
- Phase 2: Agentic Loop
- Phase 3: Hosted CI (formerly "Foundry"; reframed 2026-05-15 — see note at end)

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

## 3) Metering Economy — REFRAMED

The legacy Foundry Go backend implemented per-operation metering and a usage-breakdown endpoint, and the Python SDK exported telemetry when `LABWIRED_FOUNDRY_URL` was set. Going forward, metering is handled by the Cloudflare Worker in [`packages/api`](../../../packages/api/) (cycles-per-month quota on the Pro tier).

**Remaining:**
- Re-wire Python SDK telemetry to the Worker endpoints (currently still points at the legacy Foundry URL).
- COGS tracking per run.

## 4) Multi-Tenancy — DEPRIORITISED

The legacy Foundry backend had organisations + RBAC + audit logging. The retired product framing made these load-bearing; the CI tier currently uses per-workspace API keys without organisation primitives.

**Remaining (only if/when enterprise demands it):**
- Workspace → organisation upgrade path on the Worker side.
- SSO.
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

4. Hosted CI:
- Cloudflare Worker accepts paid API keys, enforces monthly cycle quota, and meters runs.
- Stripe checkout provisions a workspace + API key end-to-end.
- The new key is rendered in the customer's private cabinet on checkout return (Wokwi pattern) — no transactional email step.

5. Human Observer:
- One-click IDE lifecycle flow is stable.
- Core advanced debug features required by roadmap are shipped.

## Suggested Execution Priority (Highest First)

1. Ship determinism proof + compatibility matrix as release gates.
2. Close zero-touch Agentic Loop for Tier-1 targets.
3. Implement metering backend + tenancy primitives.
4. Deploy + harden the Cloudflare Worker CI control plane (`packages/api`) — Stripe webhook, key gating, run metering, cabinet-render key handoff on checkout return.

---

**Note on the Foundry reframing (2026-05-15):** The "Foundry" hosted-API framing — managed multi-tenant verification with a separate Go backend, dashboard, runs-per-month pricing — has been retired in favour of a simpler model: an open-source CLI + GitHub Action, with a paid Cloudflare Worker API gating cycles for private CI workloads. The legacy `/foundry/` Hetzner deployment remains running but is no longer the product story. See [Decommission runbook](../../ops/FOUNDRY_DECOMMISSION.md).
