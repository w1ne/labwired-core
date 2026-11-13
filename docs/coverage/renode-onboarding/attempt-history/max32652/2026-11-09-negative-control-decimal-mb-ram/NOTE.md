# max32652 — negative control: decimal-mb-ram (2026-11-09)

Run: this round's firmware and smoke against this round's descriptor with `size: 3MB` / `size: 1MB` instead of the byte counts. `parse_size` reads `MB` as decimal (1 000 000 B; `KB` is binary), so SRAM ends at 0x200F4240 and the first stack push at 0x200FEFFC faults. Result: FAIL 0/7 checks, step 0. This is why the descriptor gives 3145728B / 1048576B.
