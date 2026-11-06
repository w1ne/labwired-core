# Validation

The committed `evidence/` was produced by `scripts/onboarding_smoke.sh` using
the ESP32-C6-DevKitM-1-N4 system profile and the `riscv32imc` bare-metal
firmware. The required assertions are UART0 output, GPIO8 waveform edges,
and a decoded red pixel on the attached WS2812B model.

This is local repository-runner evidence against the in-tree C6 models. The
hosted LabWired catalog lists ESP32-C6-DevKitC-1 but does not expose a
DevKitM-1 target or a C6 hosted compile path, so there is no hosted
`labwired_verify` verdict for this exact board. This evidence is not a
hardware test.
