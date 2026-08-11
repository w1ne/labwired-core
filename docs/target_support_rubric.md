# Target support levels

When can you say a board or chip is **supported** in docs or release notes?

---

## Levels

| Level | Label | Minimum bar |
|-------|--------|-------------|
| **L0** | declared | Chip + system configs exist and validate. No smoke required. |
| **L1** | smoke-supported | Deterministic smoke script passes; reset path works; artifacts reproducible. |
| **L2** | ci-qualified | L1 + repeated CI passes + known-limitations file + instruction audit reviewed. |
| **L3** | production-ready | L2 + tier-1 peripherals proven for the documented scenarios. |

---

## Public language

Call a target **supported** only at **L1 or above**.

L0 is fine for “in progress” / experimental — say so explicitly.

---

## Evidence for L1+

1. Runnable example under `examples/` (or equivalent product lab)  
2. Deterministic CI (or local) artifact bundle: result JSON, UART log if used, fingerprint  
3. Clear stop reason and assertion outcomes  
4. **Known limitations** on the board page  

---

## Tier-1 peripherals (for L3)

Mark each `pass` / `partial` / `blocked` for the scenarios you document:

1. Clock / RCC  
2. GPIO  
3. UART  
4. Timer  
5. DMA  
6. Interrupt delivery  

L3 needs all six at **pass** for those scenarios.

---

## Promote / demote

| Change | When |
|--------|------|
| L0 → L1 | Add smoke + evidence |
| L1 → L2 | Stable CI + audits |
| L2 → L3 | Tier-1 complete |
| Demote | Smoke broken, missing limitations, or reset path regresses |

---

## Related

- [Board playbook](board_onboarding_playbook.md)
- [Fidelity](fidelity.md)
- [Chip conformance scoreboard](coverage/chip-conformance.md)
