# Hosted LabWired verifier status

Checked 2026-11-03. The hosted LabWired board catalog lists
`esp32-c6-devkitc-1` as a board that can run a supplied ELF, but returns
`compilable: false` and says there is no hosted compile path yet. No hosted
`labwired_verify` verdict is available for this target in this session.

The board-specific evidence for this round is the committed local Core CLI
onboarding smoke under `logs/` and `simulation-output/`. It asserts UART0
output against the repository's `esp32c6` model. This is not reported as a
hosted `model_verified` result or as physical hardware confirmation.
