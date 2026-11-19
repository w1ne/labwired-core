# Negative control: EFM32PG1 on the Series-0 GPIO port map (discriminating)

The round-388 firmware (`firmware-efm32pg1-smoke`, checks in
`firmware-efm32jg1-smoke/src/s1.rs`) ran against a copy of
`configs/chips/onboarding/efm32pg1.yaml` (`efm32pg1-negctl-chip.yaml`) whose
six GPIO ports are `efm32s0_gpioport` instead of `efm32s1_gpioport`. Same
bases (0x4000A000 + n * 0x30), same everything else; the system file differed
only in its `chip:` path.

Result: FAIL 3/5. On the Series-0 map 0x10/0x14 act as DOUTSET/DOUTCLR, so
`gpio reserved 0x10/0x14 no_set=0 no_clr=0 ... FAIL`; CTRL resets to 0 instead
of 0x00500050 and OVTDIS (0x28) does not exist, so
`gpio pa ctrl=00000000 ovtdis=0000 FAIL`. Summary word 0xEF31001D instead of
0xEF31003F. The Series-1 descriptor passes 5/5.

The push-pull, pull, wired and PINLOCKN checks pass on both maps: those
registers sit at the same offsets on Series 0 and Series 1.
