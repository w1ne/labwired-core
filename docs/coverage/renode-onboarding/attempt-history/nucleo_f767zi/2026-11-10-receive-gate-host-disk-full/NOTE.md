The first target-only run for the STM32v2 receiver-gate change stopped at
`build_cli` before firmware build or simulation. The host filesystem had no
space available; the recorded error and logs are preserved in this directory.
This is a host build failure, not an observed platform boot or smoke failure.

After another in-progress build on the host completed, the target-only retry
passed. Final evidence is in `docs/coverage/renode-onboarding/metrics/nucleo_f767zi/`.
