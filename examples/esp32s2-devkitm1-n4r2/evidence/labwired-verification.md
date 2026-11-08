# Hosted LabWired verifier status

Checked 2026-11-05. The hosted LabWired board catalog returned no ESP32-S2
board or MCU entry, so this target cannot receive a hosted `labwired_verify`
verdict in this session.

The evidence for this profile is the local Core CLI onboarding smoke under
`logs/` and `simulation-output/`. A passing local smoke is reported as a local
model smoke only; it is not called hosted `model_verified` or physical hardware
confirmation.
