# Negative control: EFM32ZG222 with flash at 0x08000000 (not discriminating)

Same firmware (`firmware-efm32zg222-smoke`, now linked at 0x00000000) and the
same smoke assertions, run against a temporary copy of the round-385
`configs/chips/onboarding/efm32zg222.yaml` with only `flash.base` put back to
134217728 (0x08000000).

Result: PASS 5/5, the same as the fixed descriptor. This control does NOT
distinguish the two bases. The chip descriptor defaults to
`flash_boot_alias: true`, so a non-zero flash base is also mirrored at
0x00000000 (STM32 boot behaviour). Firmware linked at 0 therefore boots
either way.

So the round-385 flash-base change makes the descriptor match Renode's
efm32zg222.repl (`flash: Memory.MappedMemory @ sysbus 0x00000000`) and the
EFM32ZG memory map. The smoke does not prove it. The old descriptor's
observable error was an extra flash window at 0x08000000 that silicon does
not have. Nothing in the smoke reads that address. To prove the fix, a smoke
would need a fault probe at 0x08000000 that is expected to BusFault/HardFault.
