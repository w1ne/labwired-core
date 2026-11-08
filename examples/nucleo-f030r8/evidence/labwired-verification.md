# Hosted LabWired catalog check

Checked 2026-11-03. The hosted board catalog returned no match for
`nucleo-f030r8`, and the hosted `labwired_verify` tool does not list this board
as a runnable target. There is no hosted oracle verdict and no hosted
`model_verified` claim for this board.

The repository's local core onboarding smoke ran the committed firmware with
the NUCLEO-F030R8 system descriptor. Its UART marker, eight PA5 GPIO edges,
and max-step stop condition passed. See [`logs/run_smoke.log`](logs/run_smoke.log),
[`simulation-output/result.json`](simulation-output/result.json), and
[`simulation-output/uart.log`](simulation-output/uart.log).
