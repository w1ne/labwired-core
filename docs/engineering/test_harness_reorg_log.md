# Test harness reorg — work log

Local branch: `chore/test-harness-organize` (do **not** push until asked).

## 2026-07-23 — Phase A

- Added `docs/engineering/test_harness.md` (layers, pass contracts, env vars,
  inventory table, roadmap).
- Inventory of `crates/core/tests`: ~6 diag / 19 differential / 17 e2e-ish /
  2 bench / 65 other.

## 2026-07-23 — Phase B

- Introduced `validation/matrix_lib/` shared by Arduino + Zephyr runners:
  - `find_labwired`, `write_test_script`, `run_labwired`, `classify_failure`
  - `render_scoreboard`
  - compile content-hash cache helpers
- Arduino `run_matrix.py`: `--sim-only`, ELF hash skip-recompile, uses matrix_lib.
- Zephyr `run_matrix.py`: uses matrix_lib for script + invoke + scoreboard.
- Removed no-op `LABWIRED_MATRIX_SPEED` reassignment; env is passed through as-is.

## 2026-07-23 — Phase C

- `boards.yaml`: required-style `budget_reason` comments/fields; ratchet some
  `max_steps` where L2 delays are short.
- Optional `led_watch: "peripheral:pin"` + matrix `--watch-gpio` + post-check
  min edge count on L2 (UART-only for RGB RMT boards without led_watch).

## Verification

- `python3 -c "import matrix_lib; ..."` import smoke
- `run_matrix.py --sim-only` on subset (STM32 + ESP) after A–C
- Full 45/45 when convenient; not pushed to remote

## 2026-07-23 — Roast follow-ups

- Removed hard-coded L0 `_pio_work/.../partitions.bin` candidates from CLI.
- L0/L1 sketch delays shortened to 1 ms (L2 already 1 ms).
- Dual-core WAITI primary batch: RTC pending-only clamp + coalesced
  `tick_elapsed(N)` under interval-1 so the path is not dead code.
- SCB permanent quantum-1 kept (logic-capture fidelity).
- PROBLEMS.md budget claims updated to match `boards.yaml`.

## 2026-07-23 — Roast round 3

- Mid-batch SW_SYS_RST: dual-core WAITI primary steps one-by-one when RTC
  present so a latched reset ends the batch early.
- nRF PIN_CNF.DIR syncs bulk DIR → LogicTap sees LED (P0.17) edges.
- C3/S3 RMT `tx_start_count` + inspect `rmt_tx` artifact; `min_rmt_tx` L2 oracle.
- Class-M dual-universe script: `scripts/matrix_speed_subset.sh`.
- Partitions resolve moved to `commands/esp32_boot_state.rs` (Phase E start).

## 2026-07-23 — CI roast follow-ups

- Path filters: onboarding + coverage-matrix fire on `crates/core/**` + `crates/cli/**`.
- Coverage-matrix: drop duplicate F103 cell; release CLI; required pass rate **1.0**.
- New `core-arduino-matrix-smoke.yml`: stm32f103, nrf52840, rp2040, esp32c3 × L0+L2.
- core-integrity: Docker runner smoke → non-required job; weekly `core-full` schedule.
- llvm-cov weekly floor 55% → 58%.
- `scripts/pre-push.sh`: fast gate (fmt/clippy/walk/RP2040/LogicTap); FULL=1 for workspace.

