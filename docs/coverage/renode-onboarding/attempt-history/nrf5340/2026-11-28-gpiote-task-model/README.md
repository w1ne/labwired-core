# nRF5340 GPIOTE0 task-mode GPIO input-latch readback (2026-11-28)

## Finding

The secure `GPIOTE0_S` window at `0x5000D000` was named
`nrf5340_gpiote_stub`. The legacy type resolver matched the `gpio` substring
and constructed the generic GPIO register model at that address. GPIOTE task
writes therefore did not update the declared GPIO0.IN register.

The chip descriptor now selects the existing Nordic `nrf52_gpiote` task/event
model and SVD IRQ 13. The task model resolves `gpio0` and `gpio1` from the
nRF5340 descriptor and latches the selected output into the GPIO model's
per-pin input register.

## Evidence

- The positive gate configures channel 0 in task mode, triggers `TASKS_SET0`,
  and checks that bit 5 is set at the SVD GPIO0.IN address (P0_S +0x10).
- The negative control builds the descriptor with its former
  `nrf5340_gpiote_stub` type; the generic GPIO fallback leaves GPIO0.IN bit 5
  low after the same task writes.
- Both results are recorded in `gpiote-test.log`.

nRF5340 is not in the 245-target Renode onboarding inventory; these board-level
checks do not change its boot/pass counts. No silicon capture is claimed.

## Limits

This check covers task-to-GPIO input-latch readback. It does not establish
external electrical pad routing, event-mode capture, full cross-peripheral
DPPI routing, TrustZone enforcement, or silicon parity. The Zephyr boot banner
remains the existing separate firmware survival gate.
