# Chip conformance — registers + behavior, same for every chip

One uniform standard, enforced in CI, with a ratchet so coverage never silently
regresses. Three layers, each reusing what already exists:

| Layer | What | Where | Runs |
|-------|------|-------|------|
| Registers (model) | SVD register coverage, never regresses | `register_coverage` | CI, all chips |
| Registers (silicon) | sim reset state == real-chip capture | `chip_conformance` L1 + `*_reset_conformance` | CI |
| Behavior | golden firmware boots + asserts its effects | `chip_conformance` L2 + `firmware_survival` / `*_exec_oracle` | CI |

The board (`docs/coverage/chip-conformance.md`) shows every chip's level; the
ratchet (`chip_conformance_ratchet`) fails CI if any chip's estate breaks, level
drops, or silicon match% falls. Missing coverage is a visible **L0/L1** cell, not
a hidden gap.

## Raising a chip with a connected board (run on this machine)

1. Add a target descriptor `scripts/hw-oracle/targets/<chip>.json` (transport +
   register windows).
2. Capture from the wired board:
   ```
   scripts/hw-oracle/hw_conform.sh scripts/hw-oracle/targets/<chip>.json out/<chip>
   ```
3. Commit `out/<chip>/reg_oracle.json` and point the chip's `reset_oracle` in
   `chip_conformance.rs` at it → the ratchet now diffs sim vs silicon (L1).
4. Add/point a `behavior_gate` (a `firmware_survival` or exec-oracle case) → L2.
5. Re-baseline: `UPDATE_CONFORMANCE_BASELINE=1 cargo test -p labwired-core --test chip_conformance`.

Transports: `openocd-esp32` (ESP32 USB-JTAG) implemented; ST-Link SWD / CMSIS-DAP
slot in as new branches in `hw_conform.sh`. The HIL runner (continuous nightly)
is a separate, later step — this path runs on demand against locally-connected chips.
