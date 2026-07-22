# Zephyr matrix — status notes

**Hard rule: NO THUNKS.** Gaps are fixed by modeling real silicon.

## Baseline (pre-matrix)

`firmware_survival` stock Zephyr 3.7 hello (and KW41Z FXOS8700): **12/12 green**.
That suite is **L0-depth only** on most chips.

## Matrix levels (this directory)

| Level | Proves |
|-------|--------|
| L0 | Kernel + console with `LW_Z0_OK` |
| L1 | `k_msleep` / system timer |
| L2 | GPIO `led0` + sleep + serial |

## Open expansion

| Item | Notes |
|------|--------|
| nRF52832 / nRF52840 | Added to boards.yaml — need first green L0–L2 |
| STM32F407 | No Zephyr board entry yet |
| ESP32 family | No Zephyr matrix path |
| KW41Z L3 sensor | FXOS8700 remains in `firmware_survival` only |

## Full matrix (2026-07-23)

**37/39 → 39/39** after L1 budget bump (`max_steps: 30M`).

Pilot green earlier: F103, L476, nRF52840 L0–L2. Full run: all 13 boards L0+L2 green; G474/H563 L1 only needed more steps (got `LW_Z1_BOOT` at 8M, `LW_Z1_OK` by 30M).

### Soft notes
- **STM32G474** L1: DBGMCU `0xE0042004` bus r/w faults logged (non-fatal; still prints OK). Optional: map DBGMCU IDCODE as RO.
- **nRF52832/40**: new vs old survival suite — full L0–L2 green.
- **STM32WBA52**: Zephyr L0–L2 green (Arduino still has no PIO board).

## Fixes landed while deepening

1. Matrix scaffold: `validation/zephyr-matrix/` L0/L1/L2 samples + `run_matrix.py`
2. System paths relative to matrix dir: `../../configs/systems/...`
3. L1 `max_steps` 8M → 30M for G474/H563 tick latency
