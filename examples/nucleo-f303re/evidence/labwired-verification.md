# Hosted LabWired catalog check

Checked 2026-11-03. The hosted board catalog returned no match for
`nucleo-f303re`; `labwired_part` returned `NOT_FOUND` for both
`NUCLEO-F303RE` and `STM32F303RE`. No hosted oracle run or
`model_verified` verdict is available for this target.

The repository's local onboarding smoke runs the committed firmware against
the NUCLEO-F303RE system descriptor. See [`logs/run_smoke.log`](logs/run_smoke.log),
[`simulation-output/result.json`](simulation-output/result.json), and
[`simulation-output/uart.log`](simulation-output/uart.log).
