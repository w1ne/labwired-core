# Verification scope — ESP8684-DevKitM-1-H4X

The repository's local onboarding smoke completed on 2026-11-09. It built the
release firmware and passed all five simulator assertions over 250,000 steps:
GPIO0, GPIO1, and GPIO8 each produced at least eight edges, and the ADC1_CH2
one-shot result at `0x3FCA0100` matched `0x47FF` (channel tag 2, raw code
2047). The run stopped at the configured step limit. See
`logs/build_firmware.log`, `logs/run_smoke.log`, and
`simulation-output/result.json`.

The LabWired hosted verifier currently accepts a fixed list of boards that
does not include ESP8684 or ESP32-C2, so no hosted `model_verified` verdict is
available for this target. This local smoke is not hosted verification and is
not a physical-board test or C2 silicon capture.
