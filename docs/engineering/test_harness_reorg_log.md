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
