# Hosted LabWired verifier result

Checked 2026-11-03. The hosted LabWired catalog did not contain a Raspberry Pi
Pico 2 W target, and its RP2350-Zero target advertises Arduino rather than the
Rust toolchain used by this smoke. A hosted Rust verification attempt on that
fallback returned `LANGUAGE_NOT_SUPPORTED`.

An Arduino fallback attempt on RP2350-Zero used the equivalent GP0 edge oracle
(`gpio_edges`, pin `GP0`, minimum 8 edges), but returned `unsupported`: the
simulator faulted on an undecoded T32 instruction at PC `0x1008` with an
undefined-instruction HardFault and observed zero edges. This is not a pass and
does not verify the committed Rust image.

The committed image instead has a passing local CLI onboarding smoke, recorded
in `logs/build_firmware.log`, `logs/run_smoke.log`, and
`simulation-output/result.json`. No hosted `model_verified` result is claimed.
