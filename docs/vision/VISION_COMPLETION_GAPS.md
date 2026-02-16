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

## Missing to Finish the Vision

## 1) Foundation Is Implemented but Not Fully Hardened

Missing:
- Determinism proof suite against real hardware baselines ("golden reference" periodic board validation) is still open in planning.
- Compatibility matrix and explicit "known gaps by MCU/peripheral" is not maintained as a release artifact.
- Run artifact standardization is not fully unified across all modes (interactive CLI vs test mode vs AI workflows).
- Current repository state includes a core build blocker in config conversion:
  - `core/crates/config/src/lib.rs:349` initializes `ChipDescriptor` without `schema_version`.

Impact:
- The "Hardware Oracle" claim is weaker without continuous determinism proof and a stable build baseline.

## 2) Agentic Loop Is In Progress, Not Zero-Touch

Missing:
- True zero-touch datasheet-to-functional-simulation path remains unmet (still requires manual corrections in practical flows).
- Advanced ingestion + behavior extraction pipeline (timing semantics, cross-peripheral side effects) is not complete.
- Model compatibility policy ("required behavior" vs "best effort") is not finalized.
- Stable external AIPi contract/package story is incomplete (docs exist, but lifecycle guarantees and versioning are not fully productized).

Impact:
- Agents can assist and accelerate, but cannot yet be trusted as fully autonomous model authors for broad device coverage.

## 3) Metering Exists as Telemetry, Not as an Economy

Missing:
- Runtime telemetry fields exist (`instructions`, `cycles`), but production metering backend is not implemented.
- No tenancy-aware quota enforcement and billing-grade accounting pipeline.
- No COGS tracking per run or pricing instrumentation needed for "Simulation Minutes" monetization.

Impact:
- The "Agent Economy" pillar is currently conceptual + local telemetry, not a deployable business system.

## 4) Foundry (Hosted Twin Service) Is Still Future-State

Missing:
- Multi-tenant control plane (Org/Project/Run hierarchy, RBAC, authn/authz).
- Secure cloud execution fabric (scheduler, isolation boundary, fleet management).
- Hosted API for on-demand twin spin-up with lifecycle management.
- Enterprise controls (audit logs, retention policy, SSO, compliance evidence pipeline/TQK workflow).

Impact:
- Phase 3 is not started as a production service. Vision remains local-tooling-centric today.

## 5) Human Observer Is Useful but Not Yet "Final Mile" Grade

Missing:
- Advanced debugger parity items still open (enhanced register semantics, reverse debugging, disassembly/profiling, RTOS-awareness).
- End-to-end "open project -> one-click run matched simulator" workflow is not complete.

Impact:
- Human verification works for demos and development, but not yet at full professional "default debugger" quality target.

## 6) Documentation and Delivery Gaps

Missing:
- A single release-grade "vision completion scoreboard" in docs has been missing (this file fills that gap).
- Landing page files currently contain unresolved merge markers:
  - `landing_page/index.html`
  - `landing_page/docs.html`
- This blocks clean public communication of status.

Impact:
- External users see mixed signals between strategy claims and operational readiness.

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

1. Restore build baseline and remove public-facing merge conflicts.
2. Ship determinism proof + compatibility matrix as release gates.
3. Close zero-touch Agentic Loop for Tier-1 targets.
4. Implement metering backend + tenancy primitives.
5. Build hosted Foundry control plane and execution service.
