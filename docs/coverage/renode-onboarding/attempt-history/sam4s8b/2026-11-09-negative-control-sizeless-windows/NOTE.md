# sam4s8b — negative control: size-less 4 KiB windows (2026-11-09)

Run: the committed smoke plus `memory_value` reads at undeclared addresses (0x400E123C PIOC_PDSR, 0x400E0C00), once against the HEAD 34106d109 descriptor (`old/`, copied as `old/chip.yaml`) and once against this round's descriptor (`new/`).

- `old/`: the reads succeed and return 0. PIOB's window (old size 4 KiB) covered PIOC, which the 64-pin SAM4S8B does not bond out, and UART1's window covered the reserved slot at 0x400E0C00; the SAM4S memory map gives UART and PIO 0x200 each.
- `new/`: every probe read fails with "Memory access violation"; the original smoke checks still pass.

A peripheral entry with no `size` gets a 4 KiB window (`push_peripheral` in crates/core/src/bus/construct.rs). Firmware that touched these addresses saw a silent zero instead of a bus fault. Audit: `scripts/audit_onboarding_window_sizes.py`.
