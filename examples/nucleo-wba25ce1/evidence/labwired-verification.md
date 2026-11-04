# Hosted LabWired verification — NUCLEO-WBA25CE1

Checked 2026-11-04. The hosted `labwired_verify` board catalog has no
NUCLEO-WBA25CE1 or STM32WBA25CE target. The repository's local
`scripts/onboarding_smoke.sh` run passed 2/2 assertions against this profile,
including at least eight GPIOA PA7 transitions and the expected max-step stop.
That local result is recorded in `run_smoke.log` and
`simulation-output/result.json`; it is not a hosted `model_verified` verdict.

No physical board capture is available. The profile only covers the modeled
RCC, GPIOA, SysTick and LD1 PA7 path; see the example README for omitted board
features.
