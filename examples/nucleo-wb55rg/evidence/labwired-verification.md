# Hosted LabWired catalog check

Checked 2026-11-03 with a diagram containing the `nucleo-wb55rg` MCU part.
`labwired_validate` returned zero errors and one warning:

```text
BOARD_NOT_RUNNABLE: diagram.board "nucleo-wb55rg" has no hosted compile/run support.
```

The hosted oracle could not run this board, so there is no hosted
`labwired_verify` verdict and no model-verified claim. The repository's local
core onboarding smoke did run the committed firmware against
`configs/systems/mb1355c.yaml`; its six UART, GPIO-edge, and stop-condition
assertions passed. See [`logs/run_smoke.log`](logs/run_smoke.log) and
[`simulation-output/result.json`](simulation-output/result.json).
