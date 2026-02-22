# Target Support Rubric

Date: February 22, 2026  
Purpose: Define when a target can be described as "supported" in docs and release notes.

## Support Levels

| Level | Label | Minimum Requirements |
|---|---|---|
| L0 | `declared` | Chip and system config files exist and validate. No smoke evidence required. |
| L1 | `smoke-supported` | Deterministic smoke script exists and passes in CI with reproducible artifacts. PC/SP reset path validated. |
| L2 | `ci-qualified` | L1 plus repeated CI pass history and known-limitations file. Unsupported-instruction audit generated and reviewed. |
| L3 | `production-ready` | L2 plus Tier-1 peripheral baseline validated (`clock/rcc`, `gpio`, `uart`, `timer`, `dma`, `irq`) and stable benchmark variance. |

## Definition of Supported

A target is "supported" for public communication only at `L1` or above.

## Mandatory Evidence for L1+

1. Runnable example script under `core/examples/`.
2. Deterministic CI artifact bundle with:
   - execution result JSON,
   - UART output log (if UART is used),
   - summary fingerprint/hash.
3. Captured stop reason and assertion outcomes.
4. Explicit known limitations section in target docs.

## Tier-1 Peripheral Checklist

Mark each as `pass`, `partial`, or `blocked`:

1. `clock/rcc`
2. `gpio`
3. `uart`
4. `timer`
5. `dma`
6. `interrupt controller / irq delivery`

`L3` requires all six at `pass` for the documented scenario set.

## Promotion Rules

1. `L0 -> L1`: add deterministic smoke and CI evidence.
2. `L1 -> L2`: add audit artifacts and trend stability over repeated runs.
3. `L2 -> L3`: complete Tier-1 peripheral checklist with no blocking gaps.

## Demotion Rules

Demote one level when any of the following occurs:

1. deterministic smoke no longer passes in CI.
2. reset path or core assertions regress.
3. known limitations are missing or stale for current behavior.

