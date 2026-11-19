# nRF52840 Renode smoke — direct-probe failures (2026-11-08)

Direct `labwired test` runs of the new `firmware-nrf52840-renode-smoke` bin
against `configs/systems/onboarding/nrf52840.yaml`, before the final pipeline
run. The files in this directory are from probe 1.

1. **max_steps 400,000** — FAIL 1/2. UARTE0 lines take ~100k cycles each at the
   configured baud. The run stopped while waiting on `EVENTS_ENDTX` after the GPIO
   line (`stalled_poll: uart0 +0x120`). This was a step budget problem, not a
   model problem. The final script uses 4,000,000.
2. **max_steps 4,000,000, first firmware** — FAIL 1/2, summary `d7`:
   ```
   gpiote out0 p0.14=00 FAIL
   rtc0 cmp=0 counter>=4 1 FAIL
   ```
   - GPIOTE: the firmware read GPIO **OUT**. On silicon a task-mode channel drives
     the pad and leaves OUT alone, so the firmware was wrong. Fixed to read IN.
   - RTC0: COUNTER advanced, but `EVENTS_COMPARE[0]` stayed 0. The model latches
     the compare event only when `EVTEN.COMPARE0` is set. The firmware now
     enables it, as nrfx does. The INTEN-only path is not exercised.
3. **IN read, P0.14 `PIN_CNF = 1` (DIR = output)** — FAIL, `gpiote out0 p0.14 in=00`.
   **Engine gap:** `Nrf52Gpio::effective_in` returns `OUT & DIR` for an output
   pin and ignores the GPIOTE pad latch. On silicon, task mode overrides the
   pin's direction and level. nrfx configures task pins as outputs, so this
   configuration matters. It is NOT claimed by the final smoke: P0.14 is left
   as an input (`PIN_CNF = 0`, buffer connected), and the model handles that
   correctly. Fixing it means changing the shared GPIO `IN` fast path.
