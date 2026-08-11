# What a green pass means

LabWired is a **deterministic digital twin**: same firmware, same system, same inputs → same result. A **pass** is a check against an explicit test or oracle — not a model saying “looks fine.”

---

## What we claim

| Claim | Meaning |
|-------|---------|
| **Register-accurate where modeled** | Firmware sees MMIO like silicon for supported blocks |
| **Same binary** | No special LabWired HAL; vendor SDK binaries |
| **Replayable** | CI and agents can re-run and get the same result |
| **Honest matrix** | Per-board ✅ / ⚠️ / ❌ — stubs are labeled |

Details: [board pages](boards/esp32c3.md) · [Target support rubric](target_support_rubric.md)

---

## What we do **not** claim

- Full analog / SPICE board physics (unless a page says otherwise)
- RF certification or perfect radio PHY
- Every peripheral on every chip at full silicon depth
- That access to an **unmodeled** register is safe — expect faults or stuck polls

Prefer a **loud fail** over a silent fake success.

---

## Evidence (not marketing)

| Scoreboard | Page |
|------------|------|
| Chip conformance | [coverage/chip-conformance](coverage/chip-conformance.md) |
| Tier-1 peripherals | [coverage/tier1-scoreboard](coverage/tier1-scoreboard.md) |
| Hardware–sim parity | [golden_reference](golden_reference.md) |
| Run limits | [limits](limits.md) |

Board pages that were checked on silicon list dates and harnesses (example: [nRF52840](boards/nrf52840.md)).

---

## Rule for agents

Do not claim the firmware is correct until **`labwired_verify`** (or a `labwired test` script) is green.

See [First agent run](agent/first-run.md).
