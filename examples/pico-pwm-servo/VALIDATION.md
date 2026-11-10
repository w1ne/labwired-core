# Validation — Raspberry Pi Pico PWM servo

Target: Raspberry Pi Pico 1 (RP2040), with an external SG90-compatible servo
signal on GP16 / header pin 21. Firmware selects PWM0A, sets DIV=125.0,
TOP=19,999 and CC_A=1,500, then leaves the 50 Hz output running.

**Local onboarding smoke: PASS (6/6 assertions).** The gate builds the firmware
and CLI and runs the deterministic RP2040 model. It requires the UART marker,
exact divider/compare/wrap register values, at least four GP16 transitions, and
the configured max-step stop. The committed result is
[`evidence/simulation-output/result.json`](evidence/simulation-output/result.json),
with stage logs under [`evidence/logs/`](evidence/logs/). This is local
digital-twin evidence, not a physical Pico capture.

The hosted `labwired_verify` result is recorded separately after running the
same behavioral oracle against the hosted Pico model; it returned
`status: model_verified` for the UART marker and four GP16 edges. The verifier
reported that PLL reconfiguration is not tracked and that VDD is assumed to be
an ideal rail. See [`evidence/hosted-verification.md`](evidence/hosted-verification.md).
Neither result claims mechanical servo movement or electrical power behavior.
