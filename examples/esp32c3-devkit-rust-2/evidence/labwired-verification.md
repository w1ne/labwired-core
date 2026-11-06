# Verification scope — ESP32-C3-DevKit-RUST-2

The onboarding evidence for this exact board is the committed local
repository smoke in this directory. It builds the Rust ELF, boots it in the
local simulator, and checks the GPIO2 waveform and decoded red NeoPixel.

The hosted LabWired catalog did not expose the exact DevKit-RUST-2 target when
this work was prepared. A hosted result for another ESP32-C3 board would not
verify this exact board profile. No hosted `model_verified` result or physical
board test is claimed here.
