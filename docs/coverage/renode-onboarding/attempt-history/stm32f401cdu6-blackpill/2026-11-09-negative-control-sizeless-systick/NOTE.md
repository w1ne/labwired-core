# stm32f401cdu6-blackpill — negative control: size-less SysTick window (2026-11-09)

Run: the committed smoke plus `memory_value` reads at 0xE000ED00 (SCB CPUID, expect 0x410FC241), 0xE000EDFC (DEMCR) and 0xE000E020 (reserved SCS gap after SysTick), once against the HEAD 76b3be811 descriptor (`old/`, copied as `old/chip.yaml`) and once against this round's descriptor (`new/`, SysTick `size: "16B"`).

- `old/`: 7/7. DEMCR and 0xE000E020 read 0. The SysTick entry had no `size`, so it got a 4 KiB window (0xE000E010..0xE000F00F). Routing is last-start-wins, so NVIC (0xE000E100) and SCB (0xE000ED00) still win their own ranges, but every SCS address that no core window covers fell into SysTick and read 0.
- `new/`: 5/7. The two gap probes fail with "Memory access violation"; CPUID and the original smoke checks pass.

CoreDebug (DHCSR..DEMCR at 0xE000EDF0) is mapped only when SWD attaches (`attach_swd_dp` in crates/core/src/system/cortex_m.rs). A size-less SysTick hid that: DEMCR writes succeeded silently on this board and faulted on boards with a sized SysTick.

Same fix on arduino_uno_r4_minima, renesas-r7fa4m1a, renesas-r7fa6m5b. Audit: `scripts/audit_onboarding_window_sizes.py` now counts the core NVIC/SCB windows as implicit neighbours.
