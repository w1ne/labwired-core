# Negative control: pre-CLINT CLI, same firmware and script

- CLI: `target/release/labwired` built 2026-11-15 03:01 UTC, before the RV32
  CLINT change (core decodes no window at 0x0200_0000).
- Firmware and script: the 2026-11-16 CLINT smoke.
- Result: FAIL 1/4 checks at step 2285, `Memory access violation at
  0x2004000` (the first `mtimecmp` read in `clint_rate_probe`).
