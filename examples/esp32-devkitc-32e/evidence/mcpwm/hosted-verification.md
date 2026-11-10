# Hosted verifier status — MCPWM smoke

The repository-local onboarding smoke passed all three assertions on
2026-11-10. The connected `labwired_verify` check did not return a firmware
verdict:

- With `language: cpp`, the runtime rejected the request and reported that the
  `esp32` target supports `arduino` and `rust`.
- Retrying with `language: arduino` was rejected by the connector schema before
  execution because that parameter only permits `c` or `cpp`.

Hosted verification is therefore inconclusive. The local smoke is the
acceptance evidence for this change; it is not a physical-board run.
