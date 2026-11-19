# Negative control: Mi-V gpioInputs without its PLIC source

- CLI: `target/debug/labwired` built 2026-11-16 04:14 UTC by the
  `--only miv_rv32` run of this change.
- Chip: `chip-without-irq29.yaml.txt`, a copy of
  `configs/chips/onboarding/miv_rv32.yaml` with the `irq: 29` line of
  `gpio_in` removed. Both CoreGPIO blocks stay mapped.
- Firmware: the 2026-11-16 Mi-V smoke with the GPIO probes (default
  features), SHA-256
  `2cad6ba463382c66e73d01743539682e2bce9eb45a1d4f1c5799617db57f6ec3`.
- Result: FAIL 2/3 checks. The pin-3 edge still latches in the block
  (`in=00000008 irq=00000008`), but PLIC source 29 never pends
  (`pending=0 meip=0 claim=00000000`). The gpioOutputs line still passes.
