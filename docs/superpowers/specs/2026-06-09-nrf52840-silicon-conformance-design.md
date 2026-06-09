# nRF52840 Silicon Conformance — F103 Parity

**Date:** 2026-06-09
**Status:** Design — pending user review
**Board under test:** Seeed XIAO nRF52840 Sense, SWD via ST-LINK V2 + OpenOCD 0.12.0
**Silicon confirmed live:** FICR `INFO.PART = 0x00052840`, variant `AAD0`, Cortex-M4 r0p1, `reset halt` OK, MSP `0x20040000`

## Goal

Bring nRF52840 onboarding to the same bar STM32F103 holds today: **every notable
peripheral modeled, swept register-by-register against real silicon, promoted into the
production chip descriptor with silicon-verified reset values, and locked behind a
ratcheted conformance gate.** When done, `onboarding/nrf52840.yaml`'s `verified: true`
is backed by an actual sim-vs-hardware diff that runs in CI-with-hardware, not an
unsubstantiated flag.

## Why this is mostly "run + promote", not "build"

nRF52840 already has a **larger** register sweep than F103 ever had:

- `core/crates/hw-oracle/tests/nrf52_onboarding_diff.rs` — 70 cases / 24 peripherals,
  each with a per-peripheral verdict enum (`Match` / `BothDisagreeWithExpect` /
  `Diverge` / `SimError`) aggregated to `MODELLED` / `WRONG_LAYOUT` / `SPEC_MISMATCH` /
  `SIM_BROKEN`.
- `core/crates/hw-oracle/tests/nrf52_mmio_diff.rs` — the 4 production peripherals
  (UART0, SPI0, GPIO0/1).
- 30 peripheral model files under `core/crates/core/src/peripherals/nrf52/`.

What's missing versus F103 is **not** the models or the sweep — it's that the sweep was
never run against silicon as a ratchet, its results were never promoted into
`configs/chips/nrf52840.yaml` (which still declares only 4 peripherals with no reset
values), there is no behavioral-digest conformance firmware, and there is no
ground-truth capture script.

## Reference templates (the F103 "gold standard" we mirror)

| Concern | F103 artifact | nRF52840 artifact to create |
|---|---|---|
| Ground-truth capture | `core/scripts/hw-capture-stm32f103.sh` | `core/scripts/hw-capture-nrf52840.sh` |
| Production descriptor | `core/configs/chips/stm32f103.yaml` (~25 periph, reset values, provenance) | promote `core/configs/chips/nrf52840.yaml` (4 → full set) |
| Behavioral digest firmware | `core/crates/firmware-f103-conformance` | `core/crates/firmware-nrf52840-conformance` |
| Conformance diff test | `core/crates/hw-oracle/tests/f103_conformance.rs` (ratchet `BASELINE_MATCHED=10/11`) | `core/crates/hw-oracle/tests/nrf52_conformance.rs` |
| Fidelity docs + audit | `core/docs/boards/stm32f103.md`, `examples/nucleo-l476rg/VALIDATION.md` | update `core/docs/boards/nrf52840.md` + new `VALIDATION.md` |

## Deliverables

### D1 — `hw-capture-nrf52840.sh` (ground-truth source)
Clone of the F103 capture script, retargeted to `interface/stlink.cfg -f target/nrf52.cfg`:
- `reset halt`, then dump every notable **reset-state** register across the 24 peripherals
  (e.g. TIMERx BITMODE/PRESCALER/CC, RTCx PRESCALER/CC, GPIOTE CONFIG, RADIO FREQUENCY/MODE,
  RNG CONFIG, WDT CRV, SAADC, PWM, QSPI, NFCT, COMP, QDEC, EGU, plus Cortex-M4 system regs
  SCB_VTOR/SysTick) and FICR identity (`INFO.PART/VARIANT/DEVICEID`).
- Output to `core/fixtures/nrf52840/hw-capture-<timestamp>/` (`st-info.txt`, `registers.txt`,
  optional `digest.txt` + `rtt.log` when the conformance firmware is flashed).
- This file is the documented, reproducible truth-set behind every reset value we commit.

### D2 — Promote verified peripherals into `configs/chips/nrf52840.yaml`
1. Run `nrf52_onboarding_diff` + `nrf52_mmio_diff` against real silicon → verdict table.
2. For each peripheral that reports `MODELLED` (sim == hw == expect): promote it from the
   current 4-entry production descriptor into the full set, each with a **reset-value block
   + provenance comment** (`# verified against nRF52840 (Seeed XIAO Sense), ST-LINK V2, 2026-06-09`),
   matching the style of `configs/chips/stm32f103.yaml` lines 79–133.
3. For `WRONG_LAYOUT` peripherals: fix the model first (see Iteration loop), then promote.
4. For `SPEC_MISMATCH` (both sim & hw disagree with the embedded `expect`): correct the
   `expect` against silicon, note the datasheet/erratum reference, then promote.
5. `SIM_BROKEN` peripherals are listed explicitly as **not yet promoted** in the docs — no
   silent omission.

### D3 — `firmware-nrf52840-conformance` + `nrf52_conformance.rs`
Cortex-M4F firmware (`thumbv7em-none-eabi`, matching `firmware-nrf52840-demo`) that drives
**deterministic** peripherals and writes an N-word behavioral digest to a fixed RAM address
ending in a `DONE` magic, mirroring `firmware-f103-conformance`:
- **Strong (fully deterministic) digest sources:** TIMER count/capture, RTC counter,
  GPIO OUT, GPIOTE+PPI+TIMER event-routing chain, ECB (fixed key+plaintext AES-128 →
  exact ciphertext digest), TEMP (range-checked).
- **Liveness-only (non-deterministic value):** RNG → assert `VALRDY`/`READY` fired, do not
  digest the random word.
- `nrf52_conformance.rs`: build firmware → run in sim until `DONE` → flash + run on HW until
  `DONE` → diff digest words → assert spot-check constants → ratchet a `BASELINE_MATCHED`
  baseline, documenting any known residual the way F103 documents the EXTI re-pend.

### D4 — Docs + provenance
- Update `core/docs/boards/nrf52840.md` with a per-peripheral fidelity table
  (Modeled / Verified / Residual).
- New `core/examples/nrf52840/VALIDATION.md` (or board example dir): per-round bug-fix audit
  trail, in the `nucleo-l476rg/VALIDATION.md` style.
- Flip `configs/onboarding/nrf52840.yaml` `verified`/`pass_rate` so the claim is backed by
  the D3 gate, not asserted.

## Iteration loop (cost control — Haiku subagents)

The register sweep itself is `cargo test` — cheap. The expensive part is the per-peripheral
**model-fix loop** when a peripheral reports `WRONG_LAYOUT`. Each such fix is independent and
narrow (one `peripherals/nrf52/<periph>.rs` file, one register layout). These are dispatched
**one Haiku subagent per peripheral**: given the silicon readback vs sim readback for that
peripheral, fix the model's register offsets/reset values/masks until that peripheral's sweep
cases go green, return the diff. The orchestrator re-runs the sweep to confirm.

## Sequencing / critical path

0. **DONE** — install openocd, confirm Seeed XIAO reachable (FICR `INFO.PART=0x52840`).
1. Run the full sweep against silicon → capture the verdict table (the "register sweep").
2. D1 capture script → record the reset-state truth-set.
3. D2 promote `MODELLED` peripherals; Haiku loop fixes `WRONG_LAYOUT`; re-sweep to green.
4. D3 conformance firmware + diff test; iterate to a documented baseline.
5. D4 docs + provenance flip.
6. Re-run `nrf52_onboarding_diff`, `nrf52_mmio_diff`, `nrf52_conformance` clean; commit.

## Risks / open items

- **UF2 bootloader.** At reset `pc=0x00000400` = the Seeed's Adafruit UF2 bootloader. The
  MMIO sweep is unaffected (works post `reset halt`). But D3 flashing must **not** mass-erase
  the bootloader: flash the conformance firmware to the application region (and set VTOR), or
  accept and document a mass-erase + re-flash-bootloader recovery step. Decision deferred to
  the plan; default = flash to app region, leave UF2 intact.
- **Port remap quirk.** The sim places GPIO P1 at `0x50001000` (not silicon `0x50000300`) to
  avoid overlapping GPIO0's 4 KB window — see `nrf52_onboarding_diff.rs` header. Capture and
  promotion must use the **silicon** addresses; the sim remap is a sim-internal detail and
  must not leak into the production descriptor's documented reset map.
- **Non-deterministic peripherals** (RNG, RADIO RSSI, TEMP jitter) are liveness-checked, not
  value-digested — keeps the conformance gate stable.
- **Probe selection.** Two ST-LINKs are attached (one likely on a Blue Pill). The capture
  script / test must select the nRF probe by serial if `target/nrf52.cfg` grabs the wrong one.

## Success criteria

- `configs/chips/nrf52840.yaml` declares the full verified peripheral set with silicon-backed
  reset values + provenance comments (parity with `stm32f103.yaml`).
- `nrf52_conformance` passes sim-vs-HW at a documented ratcheted baseline.
- `hw-capture-nrf52840.sh` reproduces the truth-set on the connected board.
- Every notable peripheral has a verdict (Verified / Residual / Not-yet-promoted) — none
  silently omitted.
- Docs + `verified` flag reflect the gate, not an assertion.
