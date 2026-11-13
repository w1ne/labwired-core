# stm32f103 — negative control: size-less 4 KiB windows (2026-11-09)

Run: the committed smoke plus `memory_value` reads at undeclared addresses (0x40011000 GPIOC, 0x40005800 I2C2), once against the HEAD 34106d109 descriptor (`old/`, copied as `old/chip.yaml`) and once against this round's descriptor (`new/`).

- `old/`: the reads succeed and return 0. GPIOB's and I2C1's default windows covered GPIOC and I2C2; RM0008 gives each block 0x400.
- `new/`: every probe read fails with "Memory access violation"; the original smoke checks still pass.

A peripheral entry with no `size` gets a 4 KiB window (`push_peripheral` in crates/core/src/bus/construct.rs). Firmware that touched these addresses saw a silent zero instead of a bus fault. Audit: `scripts/audit_onboarding_window_sizes.py`.
