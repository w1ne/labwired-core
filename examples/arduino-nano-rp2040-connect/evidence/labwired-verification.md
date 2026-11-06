# Hosted LabWired verifier result

Checked 2026-11-03. The hosted catalog does not include the Arduino Nano RP2040
Connect target. A Rust verification attempt therefore used the equivalent
`rpi-pico` reference board but failed during linking because the hosted
`cortex-m-rt` build reported missing interrupt vectors; that call produced no
verification verdict.

An Arduino GPIO sketch on the `rpi-pico` reference board then returned
`status: failed`: the GP0 edge clause observed 0 edges after the full
2,000,000-step budget, and the diagnosis said execution likely had not reached
the main loop. This is a failed hosted fallback check, not a pass for this
board or the committed Rust image. The hosted C toolchain returned
`LANGUAGE_NOT_SUPPORTED` for this board.

The 2026-11-03 hosted attempts above predate the IMU addition and do not verify
the LSM6DSOX path. For the 2026-11-06 board update, no hosted verifier verdict
was obtained. The current board-specific evidence is the committed local CLI
onboarding smoke, which loads
`configs/chips/rp2040-nano-rp2040-connect-smoke.yaml` and passes three checks:
at least eight GP0 edges, the LSM6DSOX ID/configuration/data-ready/XYZ marker,
and the expected max-step stop. This is not reported as a hosted
`model_verified` result or physical hardware confirmation.
