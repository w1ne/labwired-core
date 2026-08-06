# Fidelity — what a green pass means

LabWired is a **deterministic digital twin**: same firmware, same system, same seed → same result. A **pass** is an oracle decision, not a model opinion.

---

## What we claim

| Claim | Meaning |
|-------|---------|
| **Register-accurate where modeled** | Firmware sees MMIO like silicon for supported blocks |
| **Same binary** | No special LabWired HAL; vendor SDK binaries |
| **Replayable** | CI and agents can re-run and get the same dispose |
| **Honest matrix** | Per-board ✅ / ⚠️ / ❌ — stubs are labeled |

Per-target detail: [Boards](boards/esp32c3.md) and [Target support rubric](target_support_rubric.md).

---

## What we do **not** claim

- Full **analog / SPICE** board physics (unless a page says otherwise)  
- **RF certification** or perfect PHY  
- Every peripheral on every chip at silicon depth  
- That an **unmodeled** access is “fine” — expect faults or stuck polls  

When firmware hits unmapped or stubbed behavior, prefer **fail loud** over silent success.

---

## Live evidence (not marketing)

| Scoreboard | URL on this site |
|------------|------------------|
| Chip conformance | [coverage/chip-conformance](coverage/chip-conformance.md) |
| Tier-1 peripheral matrix | [coverage/tier1-scoreboard](coverage/tier1-scoreboard.md) |
| Hardware–sim parity | [golden_reference](golden_reference.md) |
| Run limits | [limits](limits.md) |

Silicon-verified boards cite dates and harnesses on their board page (example: [nRF52840](boards/nrf52840.md)).

---

## Agent rule

> No correctness claim without **`labwired_verify`** (or an explicit oracle assertion result).

See [First agent run](agent/first-run.md).

---

## Deeper engineering

- [Architecture](architecture.md)  
- [Hardware–sim parity](golden_reference.md)  
- [Peripheral modeling](peripherals.md)  
