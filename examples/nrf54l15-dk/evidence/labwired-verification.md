# Hosted LabWired verification attempt

Checked 2026-11-04 UTC with a GPIO oracle for P2.09 high. The hosted
`labwired_verify` request failed during its Zephyr compile setup because the
server's `/opt/west/bin/python3` is 3.11.2 while its Zephyr configuration
requires Python 3.12 or newer. It returned no verification status, so this is
not a `model_verified` result.

The repository smoke is separate evidence: [`onboarding-summary.md`](onboarding-summary.md)
records a pass with five assertions, including UART output and a captured
GPIO2:9 edge. Neither result is physical hardware verification.
