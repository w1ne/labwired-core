# Negative control: Mi-V chip renamed, same CLI, firmware and script

- CLI: `target/debug/labwired` built 2026-11-16 02:47 UTC from the Mi-V
  CLINT/PLIC change.
- Chip: `chip-renamed.yaml.txt`, a copy of `configs/chips/onboarding/miv_rv32.yaml`
  with `name: "miv_rv32_negctl"`. The core keys the CLINT and PLIC by chip
  name, so this run maps neither window. Peripherals are unchanged.
- Firmware: the 2026-11-16 Mi-V CLINT/PLIC smoke (default features), SHA-256
  `651d7a4cc5e138b38dd396fe302ec87507f639b43bf03f2c1a22a6b75858d454`.
- Result: FAIL 1/3 checks (the CLI reports step 0), `Memory access
  violation at 0x44004000` (the first `mtimecmp` read in `clint_rate_probe`).
  The UART lines before it still pass.
