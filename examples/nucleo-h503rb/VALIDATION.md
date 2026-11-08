# NUCLEO-H503RB validation

Reproduced with `scripts/onboarding_smoke.sh` on 2026-11-03. The local engine
reported `PASS 2/2 checks`: LD2 on PA5 produced at least 16 GPIO transitions,
and execution stopped at the configured step limit.

Committed artifacts are in [`evidence/`](evidence/), including the build and
run logs, JUnit assertions, simulator result, and onboarding metrics.

The tested path uses real RCC, GPIOA, and SysTick models. No zero-answering
stub is used in the descriptor. The button, VCP, USB, and other peripherals are
omitted and unsupported in this narrow profile. The simulator records
`unpowered_rail_assumed`, so board power sequencing, power-on reset, and
brown-out reset are outside this smoke.
