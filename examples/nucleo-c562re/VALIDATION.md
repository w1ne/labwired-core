# Validation: NUCLEO-C562RE

Run from the repository root with the command in [README.md](README.md). The
smoke script builds the CLI and firmware, executes the LabWired simulation,
and checks the structured result.

Evidence is committed under `evidence/`:

- `onboarding-metrics.json` and `onboarding-summary.md` record the build and
  simulation verdict.
- `logs/` contains one log per stage.
- `simulation-output/result.json` contains the UART, GPIO-edge, and stop-reason
  assertions.
- `simulation-output/` also contains the captured simulator outputs.

The exercised model is intentionally narrow. This is a local deterministic
simulation result, not a physical-board capture or a validation of peripherals
outside the profile described in [README.md](README.md).
