# Pending Silicon Verification

Standing rule: **every chip-model fix is provisional until verified against real
hardware.** A sim-consistent green in the Tier-1 matrix proves the model is
internally consistent with the fixture; only silicon breaks the circularity.
Each model-behavior change adds an entry here in the same PR; an entry closes
when its hardware verification lands (and the matrix cell graduates to the
silicon-anchored tier with the HIL workstream).

| # | Model change | PR/commit | HW verification recipe | Board | Status |
|---|---|---|---|---|---|
| 1 | Bit-band translation gated on core (M3/M4 only); H5/WBA GPIO un-shadowed | `ee1133c` | MMIO capture of GPIO word-writes at 0x4202_xxxx on silicon, replayed via the hw-oracle diff harness (pattern: `l476_mmio_diff`) | NUCLEO-H563ZI | open |
| 2 | T1 shift-immediate flags suppressed inside IT blocks | `60445bd` | Instruction-level oracle: IT-block sequences with T1 LSL/LSR/ASR, APSR captured on silicon (extend `thumb_oracles`) | any Cortex-M3/M4 board (F103 capture scripts exist) | open |
| 3 | Thumb-1 STRH/LDRSB/LDRH/LDRSH register-offset decode | `4ebed86` | Same `thumb_oracles` extension: loaded/stored values + sign-extension vs silicon | F103 (capture scripts in `scripts/`) | open |
| 4 | GDMA descriptor-walk mem-to-mem (ESP32-S3) | `fa292bd` | JTAG Unity run on the bench S3: same descriptor sequence on silicon, byte-compare (recipe: `HW_ORACLE_RESULT.md` in the platformio demo) | bench ESP32-S3 (proven setup) | open |
| 5 | ESP32-C3 TIMG0 wired to the real Timg model | `9dfe444` | T0 counter advance + latch semantics on silicon (JTAG or UART-reporting probe firmware) | **ESP32-C3 board availability unconfirmed** — blocked on hardware until then | open (blocked-on-HW) |
| 6 | RV32C compressed-branch decode fix drifted the `riscv_uart_ok` trace fingerprint | `c7148f2` | **Corrected diagnosis.** The nightly `Trace Drift Assertions` red was *not* demo-blinky pacing — the four trace cases never load blinky firmware. The gate (`scripts/trace_drift_assert.sh`) is a **sim-regression snapshot**: it fingerprints sim result+snapshot+UART against committed `examples/ci/fingerprints/`, not a silicon diff. The RVC decoder fix (#7) landed after the baseline (#76) → `riscv_uart_ok` legitimately drifted; the firmware still prints correct `RV OK` and passes. Re-baselined `riscv_uart_ok.sha256` (the only drifting case; the three ARM cases still match). | n/a (sim-only gate) | resolved (re-baselined) |
| 7 | RV32C compressed-branch offset encoding (`C.BEQZ`/`C.BNEZ`/`C.J`) + funct3=4 arithmetic group | `c7148f2` | Run the same RV32C branch/jump sequence on real RISC-V silicon (ESP32-C3 RV32IMC) and compare taken/not-taken outcomes + target PCs against the model | ESP32-C3 (board availability unconfirmed — shares #5's blocker) | open (blocked-on-HW) |

Notes:
- Recipes reuse existing machinery only: hw-oracle mmio-diff replays, `thumb_oracles`,
  the S3 JTAG Unity loop, F103 capture scripts. No new harnesses required.
- Entries #6/#7 are the live example of the rule working — a model fix (#7) tripped a
  downstream regression snapshot the moment it merged. The original #6 mis-attributed
  it to a firmware pacing change; clearing the nightly is what surfaced the true cause.
- The `Trace Drift Assertions` gate compares sim-vs-committed-sim fingerprints, so
  re-baselining after a *verified, intentional* model fix is correct snapshot
  maintenance, not a silicon claim — the silicon obligation lives in #7.
