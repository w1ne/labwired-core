Negative control (2026-11-08): `syscfg` given `profile: stm32f4` (F4 EXTICR
port codes, H = 7) instead of `stm32l1` (H = 5, G = 7). With firmware
selecting code 7 (PG on the L1) the first PH1 press pends EXTI line 1
anyway: `exti pads ... pg_quiet=0 ... FAIL`, smoke FAIL 5/7.
