# sam_e70 — negative control: size-less 4 KiB windows (2026-11-09)

Run: the committed smoke plus `memory_value` reads at undeclared addresses (0x400E1900, 0x400E2000), once against the HEAD 34106d109 descriptor (`old/`, copied as `old/chip.yaml`) and once against this round's descriptor (`new/`).

- `old/`: the reads succeed and return 0. UART1's window covered the system controller slots after WDT, and UART4's window covered 0x400E2000 onward. UART0 ends at CHIPID (0x400E0940), so its size is 0x140.
- `new/`: every probe read fails with "Memory access violation"; the original smoke checks still pass.

A peripheral entry with no `size` gets a 4 KiB window (`push_peripheral` in crates/core/src/bus/construct.rs). Firmware that touched these addresses saw a silent zero instead of a bus fault. Audit: `scripts/audit_onboarding_window_sizes.py`.
