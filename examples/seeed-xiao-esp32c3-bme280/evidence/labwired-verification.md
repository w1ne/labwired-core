# Hosted verifier availability

Checked on 2026-11-06. The hosted LabWired board catalog did not contain
`seeed-xiao-esp32c3`: `labwired_describe` returned `NOT_FOUND`, and filtering
the board catalog for `xiao` returned only the XIAO nRF52840 Sense. Therefore
there is no hosted `labwired_verify` verdict for this exact board profile.

The committed evidence is the repository's local onboarding smoke in
`simulation-output/result.json`, which passed its three declared assertions.
That result is not a hosted model-verification verdict or a physical-hardware
test.
