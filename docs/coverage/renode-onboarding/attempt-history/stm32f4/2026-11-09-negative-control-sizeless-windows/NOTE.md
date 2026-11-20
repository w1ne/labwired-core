# stm32f4 — negative control: size-less 4 KiB windows (2026-11-09)

Run: the committed smoke plus `memory_value` reads at undeclared addresses (0x40020414 GPIOB_ODR, 0x40020814 GPIOC_ODR, 0x40004814 USART3_BRR), once against the HEAD 34106d109 descriptor (`old/`, copied as `old/chip.yaml`) and once against this round's descriptor (`new/`).

- `old/`: the reads succeed and return 0. GPIOA's and USART2's default windows covered GPIOB/GPIOC and USART3..UART5; RM0090 gives each block 0x400.
- `new/`: every probe read fails with "Memory access violation"; the original smoke checks still pass.

A peripheral entry with no `size` gets a 4 KiB window (`push_peripheral` in crates/core/src/bus/construct.rs). Firmware that touched these addresses saw a silent zero instead of a bus fault. Audit: `scripts/audit_onboarding_window_sizes.py`.
