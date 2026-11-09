# efm32lg995 — negative control: old firmware on the corrected memory map (2026-11-09)

Run: firmware-efm32lg995-smoke at 53826d2c0 (memory.x FLASH 1024K, RAM 124K,
summary 0x2001FF00) against this round's descriptor (256 KiB flash, 32 KiB
SRAM: Renode efm32lg995.repl overrides the 1 MiB / 128 KiB of the
efm32gg942.repl it includes). Run via
`run_renode_onboarding_inventory.py --only efm32lg995`.

Result: FAIL at run_smoke, step 0: `Memory access violation at 0x2001efe0`.
The old image puts its initial SP at 0x2001F000, outside the real 32 KiB
SRAM. The old descriptor only "passed" because it declared 128 KiB of SRAM
that the EFM32LG995F256 does not have.
