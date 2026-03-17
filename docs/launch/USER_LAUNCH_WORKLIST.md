[← Back to Hub](../README.md)

# User Launch Worklist

This document defines what LabWired must do from a user perspective before we call the product launch-ready.

## Primary Promise

LabWired lets embedded developers run and debug firmware deterministically without requiring physical hardware.

## Primary User Workflow

For launch, the core workflow must be:

1. Install prerequisites and build LabWired.
2. Run one bundled firmware example successfully.
3. See deterministic output and machine-readable artifacts.
4. Open the same example in VS Code and debug it.
5. Reuse the same workflow in CI.

Hosted Foundry is valuable, but it is a secondary or beta workflow until the local and IDE loop is fully coherent.

## Must Fix Before Launch

### 1. Single Start Path

- A new user must know exactly where to start.
- Root docs must point to one canonical quickstart, not multiple competing product narratives.
- The quickstart must be tested from a clean checkout.

### 2. First Run Must Succeed Quickly

- A user must be able to run a bundled example in under 10 minutes.
- The commands must work exactly as written.
- Expected output must be shown in docs so success is obvious.

### 3. Clear Support Boundaries

- Publish which MCU families, boards, and peripheral classes are reliable today.
- Mark experimental targets clearly.
- Publish known gaps that can affect trust in results.

### 4. Deterministic Evidence

- The user must be able to see artifacts such as `result.json`, `snapshot.json`, and UART/log output.
- Release notes must include determinism evidence and compatibility evidence, not only feature claims.

### 5. VS Code Happy Path

- Opening a shipped example and debugging it in VS Code must be straightforward.
- Breakpoints, stepping, registers, memory, and peripheral inspection must work for at least one recommended example.
- The extension should not require undocumented manual setup for the recommended path.

### 6. Troubleshooting

- Common failures need direct fixes:
  - missing toolchains
  - missing firmware targets
  - unsupported instructions
  - stale system YAML
  - VS Code launch configuration mismatch

## Can Ship As Beta

These can be public, but only if labeled clearly as beta or experimental:

- Hosted Foundry verification service
- Foundry billing and quota purchase flows
- AI-assisted datasheet-to-model generation
- Broader board catalog beyond the recommended launch targets

## Defer

These should not block the first user-facing launch:

- Full metering economy and billing-grade accounting
- Enterprise controls such as RBAC, SSO, and compliance exports
- Fully autonomous zero-touch datasheet ingestion across broad target coverage
- Advanced debugger parity items such as reverse debugging

## High-Impact Docs To Keep Current

- `README.md`
- `DEVELOPMENT.md`
- `docs/README.md`
- `docs/specs/compatibility_matrix.md`
- `docs/user_flows.md`
- `vscode/README.md`
- `foundry/frontend/README.md`

## Cleanup Rule

If a document describes a different primary product than the user workflow above, either:

- update it to match the current launch story, or
- keep it as strategy/internal material and label it clearly so users do not mistake it for the main product path.
